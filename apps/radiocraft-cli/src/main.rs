//! `radiocraft-cli`: RadioCraft without a window. It runs the same commands as the app.
//!
//! - `radiocraft-cli commands`: list the commands.
//! - `radiocraft-cli exec <command> [json]`: run one command on an empty session.
//! - `radiocraft-cli script <script.json>`: run `{"commands": [{"command", "params"}...]}` in order.
//! - `radiocraft-cli plan <plan.dxf|.obj|.stl|.sc> [options]`: open a plan, place routers, report.

use std::process::ExitCode;

use radiocraft_engine::{COMMANDS, Engine};
use serde_json::{Value, json};

const USAGE: &str = "usage:
  radiocraft-cli commands
  radiocraft-cli exec <command> [json-params]
  radiocraft-cli script <script.json>
  radiocraft-cli plan <plan.dxf|.obj|.stl|.sc> [--router x,y[,height]]... [--device id | --eirp dBm] [--devices file.json]
                      [--band 2.4|5|6] [--slice mm] [--viewpoint top|front|…|view:<name>] [--material source=material]... [--png out.png] [--save project.rcraft]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string())
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("radiocraft-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<Value, String> {
    let mut engine = Engine::new();
    match args.first().map(String::as_str) {
        Some("commands") => Ok(json!(
            COMMANDS
                .iter()
                .map(|(id, doc)| json!({"id": id, "params": doc}))
                .collect::<Vec<_>>()
        )),
        Some("exec") => {
            let command = args.get(1).ok_or(USAGE)?;
            let params: Value = match args.get(2) {
                Some(text) => serde_json::from_str(text).map_err(|e| format!("params: {e}"))?,
                None => json!({}),
            };
            engine.execute(command, &params).map_err(|e| e.to_string())
        }
        Some("script") => {
            let path = args.get(1).ok_or(USAGE)?;
            let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
            let script: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
            let commands = script
                .get("commands")
                .and_then(Value::as_array)
                .ok_or("a script is {\"commands\": [{\"command\", \"params\"}...]}")?;
            let mut results = Vec::new();
            for (i, step) in commands.iter().enumerate() {
                let command = step
                    .get("command")
                    .and_then(Value::as_str)
                    .ok_or(format!("step {i}: no command"))?;
                let params = step.get("params").cloned().unwrap_or(json!({}));
                results.push(
                    engine
                        .execute(command, &params)
                        .map_err(|e| format!("step {i}: {e}"))?,
                );
            }
            Ok(json!(results))
        }
        Some("plan") => plan(&mut engine, args.get(1..).unwrap_or(&[])),
        _ => Err(USAGE.to_string()),
    }
}

fn exec(engine: &mut Engine, command: &str, params: Value) -> Result<Value, String> {
    engine.execute(command, &params).map_err(|e| e.to_string())
}

fn plan(engine: &mut Engine, args: &[String]) -> Result<Value, String> {
    let path = args.first().ok_or(USAGE)?;
    let mut routers: Vec<Vec<f64>> = Vec::new();
    let mut power = json!({});
    let mut materials: Vec<(String, String)> = Vec::new();
    let (mut png, mut save) = (None, None);
    let mut settings = serde_json::Map::new();
    let mut it = args.iter().skip(1);
    while let Some(flag) = it.next() {
        let value = it.next().cloned().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--router" => {
                let xs: Result<Vec<f64>, _> =
                    value.split(',').map(|x| x.trim().parse::<f64>()).collect();
                let xs = xs.map_err(|_| format!("--router {value}: x,y[,height] in mm"))?;
                if !(2..=3).contains(&xs.len()) {
                    return Err(format!("--router {value}: x,y[,height] in mm"));
                }
                routers.push(xs);
            }
            "--device" => power = json!({"device": value}),
            "--eirp" => {
                power = json!({"eirp_dbm": value.parse::<f64>().map_err(|_| "--eirp takes dBm")?})
            }
            "--devices" => {
                exec(engine, "devices.load", json!({"path": value}))?;
            }
            "--band" => {
                settings.insert("band".into(), json!(value));
            }
            "--viewpoint" => {
                settings.insert("viewpoint".into(), json!(value));
            }
            "--slice" => {
                settings.insert(
                    "slice_height".into(),
                    json!(value.parse::<f64>().map_err(|_| "--slice takes mm")?),
                );
            }
            "--material" => {
                let (source, material) = value
                    .split_once('=')
                    .ok_or(format!("--material {value}: source=material"))?;
                materials.push((source.to_string(), material.to_string()));
            }
            "--png" => png = Some(value),
            "--save" => save = Some(value),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    // The slice height and viewpoint come first: a model is cut when it opens.
    let mut open = json!({"path": path});
    for key in ["slice_height", "viewpoint"] {
        if let Some(v) = settings.remove(key) {
            open[key] = v;
        }
    }
    let info = exec(engine, "plan.open", open)?;
    exec(engine, "settings.set", Value::Object(settings))?;
    for (source, material) in &materials {
        let m: Value = serde_json::from_str(material).unwrap_or_else(|_| json!(material));
        exec(
            engine,
            "material.set",
            json!({"source": source, "material": m}),
        )?;
    }
    for r in &routers {
        let mut p = power.clone();
        p["at"] = json!([r.first(), r.get(1)]);
        if let Some(h) = r.get(2) {
            p["height"] = json!(h);
        }
        exec(engine, "router.add", p)?;
    }
    let coverage = exec(engine, "coverage.compute", json!({}))?;
    let mut out = json!({"plan": info, "coverage": coverage});
    if let Some(png) = png {
        out["png"] = exec(engine, "export.png", json!({"path": png}))?;
    }
    if let Some(save) = save {
        out["project"] = exec(engine, "project.save", json!({"path": save}))?;
    }
    Ok(out)
}
