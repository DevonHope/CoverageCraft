//! RadioCraft's engine on real files: CADCraft's apartment plan, and a model as SolidCraft exports it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Tests fail by panicking.

use radiocraft_engine::Engine;
use serde_json::{Value, json};

fn fixture(name: &str) -> String {
    format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, &params)
        .unwrap_or_else(|err| panic!("{command} {params}: {err}"))
}

fn source<'a>(info: &'a Value, name: &str) -> &'a Value {
    info["sources"]
        .as_array()
        .and_then(|s| s.iter().find(|s| s["name"] == name))
        .unwrap_or_else(|| panic!("no source {name}: {info}"))
}

#[test]
fn cadcrafts_apartment_plan_gives_walls_glazing_and_a_covered_floor() {
    let mut e = Engine::new();
    let info = run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    // The layers' names choose their materials; furniture and notes are left out.
    assert_eq!(source(&info, "A-WALL")["material"], "auto");
    assert_eq!(source(&info, "A-GLAZ")["material"], "interior-glass");
    assert_eq!(source(&info, "A-DOOR")["material"], "wood-door");
    assert_eq!(source(&info, "A-FURN")["material"], "ignore");
    let b = info["bounds"].as_array().unwrap();
    let (w, h) = (
        b[2].as_f64().unwrap() - b[0].as_f64().unwrap(),
        b[3].as_f64().unwrap() - b[1].as_f64().unwrap(),
    );
    // An apartment, in millimetres: metres across, not kilometres or millimetres.
    assert!(
        (3000.0..60000.0).contains(&w) && (3000.0..60000.0).contains(&h),
        "{w} × {h}"
    );
    let centre = [
        b[0].as_f64().unwrap() + w / 2.0,
        b[1].as_f64().unwrap() + h / 2.0,
    ];

    run(
        &mut e,
        "router.add",
        json!({"at": centre, "height": 2400, "device": "generic-ceiling-ap"}),
    );
    let middle = run(&mut e, "coverage.compute", json!({}));
    assert!(middle["walls"].as_u64().unwrap() > 0, "{middle}");
    assert!(middle["floor_m2"].as_f64().unwrap() > 5.0, "{middle}");
    let good = middle["pct_at_least_60"].as_f64().unwrap();
    assert!((0.0..=100.0).contains(&good));

    // Moved to a corner, the router covers less of the floor.
    run(
        &mut e,
        "router.move",
        json!({"id": 1, "at": [b[0].as_f64().unwrap() + 300.0, b[1].as_f64().unwrap() + 300.0]}),
    );
    let corner = run(&mut e, "coverage.compute", json!({}));
    assert!(
        corner["median_dbm"].as_f64().unwrap() < middle["median_dbm"].as_f64().unwrap(),
        "{middle} {corner}"
    );

    // Walls of concrete instead of the thickness guess cost the far rooms more.
    run(
        &mut e,
        "material.set",
        json!({"source": "A-WALL", "material": "concrete"}),
    );
    let concrete = run(&mut e, "coverage.compute", json!({}));
    assert!(
        concrete["p5_dbm"].as_f64().unwrap() <= corner["p5_dbm"].as_f64().unwrap(),
        "{corner} {concrete}"
    );
}

/// A room 4 m × 3 m with 150 mm walls and a glass pane, written as SolidCraft writes OBJ: one
/// `o` per body, Z up, millimetres, and a normal on every face corner (`f a//n`), so corners that
/// share a position have different normals.
fn solidcraft_obj() -> String {
    let mut s = String::from("# SolidCraft OBJ export (mm)\n");
    let mut base = 1;
    let mut normal = 0;
    let mut cuboid = |name: &str, x0: f64, y0: f64, x1: f64, y1: f64| {
        s += &format!("o {name}\n");
        for (x, y, z) in [
            (x0, y0, 0.0),
            (x1, y0, 0.0),
            (x1, y1, 0.0),
            (x0, y1, 0.0),
            (x0, y0, 2500.0),
            (x1, y0, 2500.0),
            (x1, y1, 2500.0),
            (x0, y1, 2500.0),
        ] {
            s += &format!("v {x} {y} {z}\n");
        }
        for (k, f) in [
            [1, 3, 2],
            [1, 4, 3],
            [5, 6, 7],
            [5, 7, 8],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 4, 8],
            [3, 8, 7],
            [4, 1, 5],
            [4, 5, 8],
        ]
        .iter()
        .enumerate()
        {
            // One normal per pair of triangles (a face of the box).
            if k % 2 == 0 {
                s += "vn 0 0 1\n";
                normal += 1;
            }
            s += &format!(
                "f {}//{normal} {}//{normal} {}//{normal}\n",
                f[0] + base - 1,
                f[1] + base - 1,
                f[2] + base - 1
            );
        }
        base += 8;
    };
    cuboid("Wall S", -150.0, -150.0, 4150.0, 0.0);
    cuboid("Wall N", -150.0, 3000.0, 4150.0, 3150.0);
    cuboid("Wall W", -150.0, 0.0, 0.0, 3000.0);
    cuboid("Wall E", 4000.0, 0.0, 4150.0, 3000.0);
    cuboid("Glass pane", 2000.0, 500.0, 2006.0, 2500.0);
    cuboid("Sofa", 500.0, 500.0, 1500.0, 1200.0);
    s
}

#[test]
fn a_solidcraft_model_is_cut_at_the_slice_height() {
    let dir = std::env::temp_dir().join(format!("radiocraft-it-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("room.obj");
    std::fs::write(&path, solidcraft_obj()).unwrap();
    let mut e = Engine::new();
    let info = run(
        &mut e,
        "plan.open",
        json!({"path": path.to_string_lossy(), "slice_height": 1000}),
    );
    assert_eq!(source(&info, "Glass pane")["material"], "interior-glass");
    assert_eq!(source(&info, "Sofa")["material"], "ignore");
    assert_eq!(source(&info, "Wall S")["shapes"], 1);
    run(
        &mut e,
        "router.add",
        json!({"at": [1000, 1500], "height": 1000, "eirp_dbm": 20}),
    );
    // Behind the pane: 3 dB of interior glass at 5 GHz on the way.
    let behind = run(&mut e, "coverage.at", json!({"at": [3000, 1500]}));
    assert_eq!(behind["bands"]["5"]["walls_db"], 3.0, "{behind}");
    let stats = run(&mut e, "coverage.compute", json!({}));
    assert_eq!(stats["floor"], "enclosed");
    assert!(
        (10.0..12.5).contains(&stats["floor_m2"].as_f64().unwrap()),
        "{stats}"
    );
    // Cut above the walls, nothing is left to analyse.
    assert!(
        e.execute("settings.set", &json!({"slice_height": 5000}))
            .is_ok()
    );
    assert!(e.execute("coverage.compute", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn projects_save_and_open_again() {
    let dir = std::env::temp_dir().join(format!("radiocraft-project-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    run(
        &mut e,
        "material.set",
        json!({"source": "A-GLAZ", "material": "low-e-glass"}),
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [5000, 4000], "height": 2400, "name": "Hall", "eirp_dbm": {"2.4": 20, "5": 23}}),
    );
    run(
        &mut e,
        "settings.set",
        json!({"band": "2.4", "grid_step": 150}),
    );
    let before = run(&mut e, "coverage.compute", json!({}));
    let project = dir.join("home.rcraft");
    run(
        &mut e,
        "project.save",
        json!({"path": project.to_string_lossy()}),
    );
    let mut again = Engine::new();
    run(
        &mut again,
        "project.open",
        json!({"path": project.to_string_lossy()}),
    );
    assert_eq!(before, run(&mut again, "coverage.compute", json!({})));
    let routers = run(&mut again, "router.list", json!({}));
    assert_eq!(routers[0]["name"], "Hall");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two routers on the same 5 GHz channel, one on another: the maps, the probe and the figures
/// agree, switching maps changes no figure, and the leakage map leaves the floor's figures alone.
#[test]
fn channels_maps_and_leakage() {
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    run(
        &mut e,
        "material.set",
        json!({"source": "A-WALL", "material": "concrete"}),
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [2000, 2000], "height": 2400, "device": "generic-ceiling-ap", "name": "West", "channels": {"5": {"channel": 36, "width": 80}}}),
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [10000, 6000], "height": 2400, "device": "generic-ceiling-ap", "name": "East", "channels": {"5": {"channel": 44, "width": 20}}}),
    );
    let base = run(&mut e, "coverage.compute", json!({}));
    // 36/80 covers 44: everywhere one is heard over the other, they share airtime.
    assert!(
        base["pct_no_channel_overlap"].as_f64().unwrap() < 100.0,
        "{base}"
    );
    assert!(base["pct_second_at_least_67"].as_f64().is_some(), "{base}");
    assert!(base["median_rate_mbps"].as_f64().unwrap() > 0.0, "{base}");
    // Between the two, the probe names the other router as on an overlapping channel.
    let probe = run(&mut e, "coverage.at", json!({"at": [6000, 4000]}));
    let five = &probe["bands"]["5"];
    assert_eq!(five["routers"].as_array().map(Vec::len), Some(2), "{probe}");
    assert_eq!(five["channel_overlap"], 1, "{probe}");
    assert!(five["rate_mbps"].as_f64().unwrap() > 0.0, "{probe}");
    // Moving East to channel 149 clears the overlap.
    run(
        &mut e,
        "router.set",
        json!({"id": 2, "channels": {"5": 149}}),
    );
    let apart = run(&mut e, "coverage.compute", json!({}));
    assert_eq!(apart["pct_no_channel_overlap"], 100.0, "{apart}");
    // Every map is drawn from the same signals: the figures don't move.
    for view in ["second", "serving", "overlap", "rate", "signal"] {
        run(&mut e, "settings.set", json!({"view": view}));
        let again = run(&mut e, "coverage.compute", json!({}));
        assert_eq!(again["pct_at_least_67"], apart["pct_at_least_67"], "{view}");
        assert_eq!(again["floor_m2"], apart["floor_m2"], "{view}");
    }
    // The leakage map works out the cells around the floor too, without changing the floor's.
    run(&mut e, "settings.set", json!({"view": "leakage"}));
    let leak = run(&mut e, "coverage.compute", json!({}));
    assert_eq!(leak["floor_m2"], apart["floor_m2"], "{leak}");
    assert_eq!(leak["pct_at_least_67"], apart["pct_at_least_67"], "{leak}");
    assert!(
        leak["leakage"]["m2_at_least_75"].as_f64().unwrap() > 0.0,
        "{leak}"
    );
}

/// The report holds every band's maps, escapes what users type, names the plan by its file only,
/// and leaves the shown band and map as they were.
#[test]
fn the_report_is_one_html_page() {
    let dir = std::env::temp_dir().join(format!("radiocraft-report-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [2500, 2500], "height": 2400, "device": "generic-router", "name": "Hall <main>", "channels": {"5": 36, "2.4": 1}}),
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [9500, 6000], "height": 2400, "device": "generic-router", "name": "Den", "channels": {"5": 40}}),
    );
    run(
        &mut e,
        "settings.set",
        json!({"band": "2.4", "view": "rate"}),
    );
    let path = dir.join("report.html");
    let r = run(
        &mut e,
        "report.export",
        json!({"path": path.to_string_lossy(), "title": "Flat & co", "notes": "Router <b>here</b>"}),
    );
    // 2.4 and 5 GHz; each with signal, second, serving, overlap, rate and leakage.
    assert_eq!(r["bands"], 2, "{r}");
    assert_eq!(r["maps"], 12, "{r}");
    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains("<title>Flat &amp; co</title>"));
    assert!(html.contains("Hall &lt;main&gt;") && html.contains("Router &lt;b&gt;here&lt;/b&gt;"));
    assert!(html.contains("Plan: cadcraft-apartment.dxf"));
    assert!(
        !html.contains(env!("CARGO_MANIFEST_DIR")),
        "the report names folders"
    );
    assert_eq!(html.matches("data:image/png;base64,").count(), 12);
    assert_eq!(e.settings().band.name(), "2.4");
    assert_eq!(e.settings().view, radiocraft_engine::radio::View::Rate);
    assert!(
        e.execute(
            "report.export",
            &json!({"path": dir.join("r.pdf").to_string_lossy()})
        )
        .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A suggested spot beats a corner, is the same every time, and a second router helps more.
#[test]
fn suggested_spots_beat_a_corner() {
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    run(
        &mut e,
        "material.set",
        json!({"source": "A-WALL", "material": "concrete"}),
    );
    let one = run(
        &mut e,
        "router.suggest",
        json!({"device": "generic-router", "height": 900}),
    );
    assert_eq!(
        one,
        run(
            &mut e,
            "router.suggest",
            json!({"device": "generic-router", "height": 900})
        )
    );
    let at = one["spots"][0]["at"].clone();
    run(
        &mut e,
        "router.add",
        json!({"at": [300, 300], "height": 900, "device": "generic-router"}),
    );
    let corner = run(&mut e, "coverage.compute", json!({}))["pct_at_least_67"]
        .as_f64()
        .unwrap();
    run(&mut e, "router.move", json!({"id": 1, "at": at}));
    let suggested = run(&mut e, "coverage.compute", json!({}))["pct_at_least_67"]
        .as_f64()
        .unwrap();
    assert!(suggested > corner, "{corner} → {suggested} at {at}");
    // With add, the next one is placed, and it helps.
    let two = run(
        &mut e,
        "router.suggest",
        json!({"device": "generic-router", "height": 900, "add": true}),
    );
    assert!(two["spots"][0]["id"].is_number(), "{two}");
    let both = run(&mut e, "coverage.compute", json!({}))["pct_at_least_67"]
        .as_f64()
        .unwrap();
    assert!(both >= suggested, "{suggested} → {both}");
    assert_eq!(e.routers().len(), 2);
    assert!(e.execute("router.suggest", &json!({"count": 9})).is_err());
    assert!(e.execute("router.suggest", &json!({"count": 1.5})).is_err());
}

/// Made-up measurements from a model with a known exponent and offsets: the comparison shows the
/// default model's error, and calibrating finds the exponent and offsets again.
#[test]
fn calibration_finds_a_known_model() {
    let dir = std::env::temp_dir().join(format!("radiocraft-survey-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let setup = |e: &mut Engine| {
        run(
            e,
            "plan.open",
            json!({"path": fixture("cadcraft-apartment.dxf")}),
        );
        run(
            e,
            "material.set",
            json!({"source": "A-WALL", "material": "concrete"}),
        );
        run(
            e,
            "router.add",
            json!({"at": [2500, 2500], "height": 2400, "device": "generic-router", "name": "Hall"}),
        );
    };
    // The "truth": exponent 3.0, 5 GHz 4 dB lower and 2.4 GHz 2 dB higher than the plain model.
    let mut truth = Engine::new();
    setup(&mut truth);
    run(
        &mut truth,
        "settings.set",
        json!({"exponent": 3.0, "offset_db": {"5": -4, "2.4": 2}}),
    );
    let mut csv = String::from("x,y,band,rssi_dbm,router\n");
    for x in (1000..12000).step_by(1500) {
        for y in (1000..8000).step_by(1500) {
            let at = run(&mut truth, "coverage.at", json!({"at": [x, y]}));
            for band in ["2.4", "5"] {
                let v = at["bands"][band]["rssi_dbm"].as_f64().unwrap();
                csv += &format!("{x},{y},{band},{v},Hall\n");
            }
        }
    }
    let path = dir.join("survey.csv");
    std::fs::write(&path, &csv).unwrap();
    let mut e = Engine::new();
    setup(&mut e);
    let n = run(
        &mut e,
        "survey.import",
        json!({"path": path.to_string_lossy()}),
    );
    assert_eq!(n["points"], 80);
    let before = run(&mut e, "survey.compare", json!({}));
    assert!(
        before["bands"]["5"]["rms_db"].as_f64().unwrap() > 3.0,
        "{before}"
    );
    // Without apply, nothing changes.
    let fit = run(&mut e, "survey.calibrate", json!({}));
    assert_eq!(e.settings().exponent, 2.2);
    assert!(
        (fit["exponent"].as_f64().unwrap() - 3.0).abs() < 0.05,
        "{fit}"
    );
    assert!(
        (fit["offset_db"]["5"].as_f64().unwrap() + 4.0).abs() < 0.3,
        "{fit}"
    );
    assert!(
        (fit["offset_db"]["2.4"].as_f64().unwrap() - 2.0).abs() < 0.3,
        "{fit}"
    );
    assert!(fit["after"]["5"]["rms_db"].as_f64().unwrap() < 0.2, "{fit}");
    run(&mut e, "survey.calibrate", json!({"apply": true}));
    assert!((e.settings().exponent - 3.0).abs() < 0.05);
    // The survey and the calibration are saved with the project.
    let project = dir.join("p.rcraft");
    run(
        &mut e,
        "project.save",
        json!({"path": project.to_string_lossy()}),
    );
    let mut again = Engine::new();
    run(
        &mut again,
        "project.open",
        json!({"path": project.to_string_lossy()}),
    );
    assert_eq!(again.survey().len(), 80);
    assert_eq!(again.settings().offset_db, e.settings().offset_db);
    assert!(
        again
            .execute(
                "survey.add",
                &json!({"at": [0, 0], "band": "5", "rssi_dbm": 10})
            )
            .is_err()
    );
    assert!(
        again
            .execute("survey.import", &json!({"path": project.to_string_lossy()}))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project saved before channels and maps existed opens, with them unset.
#[test]
fn an_older_project_opens() {
    let dir = std::env::temp_dir().join(format!("radiocraft-old-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.rcraft");
    let old = json!({
        "format": "radiocraft-project/1",
        "plan": fixture("cadcraft-apartment.dxf"),
        "materials": [{"source": "A-WALL", "kind": "layer", "material": "concrete"}],
        "routers": [{"name": "Hall", "at": [5000, 4000], "height": 2400, "power": {"device": "generic-router"}}],
        "settings": {"band": "5", "grid_step": 100, "client_height": 1000, "exponent": 2.2, "slice_height": 1000, "cap_eirp": true, "close_gaps": 1000, "up": "z"},
        "devices_file": null,
    });
    std::fs::write(&path, old.to_string()).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "project.open",
        json!({"path": path.to_string_lossy()}),
    );
    let routers = run(&mut e, "router.list", json!({}));
    assert_eq!(routers[0]["channels"], json!({}));
    assert_eq!(e.settings().view, radiocraft_engine::radio::View::Signal);
    assert!(run(&mut e, "coverage.compute", json!({}))["pct_at_least_67"].is_number());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_commands_are_refused() {
    let mut e = Engine::new();
    for (command, params) in [
        ("plan.open", json!({"path": "/nonexistent.dxf"})),
        ("plan.open", json!({})),
        (
            "plan.open",
            json!({"path": fixture("cadcraft-apartment.dxf"), "up": "w"}),
        ),
        ("router.add", json!({"at": [f64::MAX, 0]})),
        ("router.add", json!({"at": [0, 0], "eirp_dbm": 999})),
        ("router.add", json!({"at": [0, 0], "device": "nope"})),
        ("router.move", json!({"id": 99, "at": [0, 0]})),
        ("settings.set", json!({"band": "60"})),
        ("settings.set", json!({"grid_step": 0})),
        ("settings.set", json!({"view": "heat"})),
        ("settings.set", json!({"offset_db": {"5": 99}})),
        ("settings.set", json!({"offset_db": 3})),
        ("survey.compare", json!({})),
        ("survey.calibrate", json!({})),
        (
            "survey.add",
            json!({"at": [0, 0], "band": "60", "rssi_dbm": -50}),
        ),
        ("survey.import", json!({"path": "/nonexistent.csv"})),
        ("settings.set", json!({"client_streams": 5})),
        ("settings.set", json!({"client_streams": 1.5})),
        ("router.add", json!({"at": [0, 0], "channels": {"5": 37}})),
        (
            "router.add",
            json!({"at": [0, 0], "channels": {"2.4": {"channel": 6, "width": 40}}}),
        ),
        ("router.add", json!({"at": [0, 0], "channels": {"7": 1}})),
        ("router.add", json!({"at": [0, 0], "channels": [36]})),
        (
            "router.add",
            json!({"at": [0, 0], "channels": {"6": {"channel": 229, "width": 160}}}),
        ),
        (
            "router.add",
            json!({"at": [0, 0], "channels": {"5": {"channel": 65536}}}),
        ),
        (
            "material.set",
            json!({"source": "A-WALL", "material": "brick"}),
        ),
        ("export.png", json!({"path": "/tmp/x.txt"})),
        ("nope", json!({})),
        ("plan.info", json!([1, 2])),
    ] {
        assert!(e.execute(command, &params).is_err(), "{command} {params}");
    }
    // Garbage as a DXF is an error, not a crash.
    let dir = std::env::temp_dir().join(format!("radiocraft-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in [
        (
            "x.dxf",
            b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\nnan\n".to_vec(),
        ),
        ("y.obj", b"f 1 2 3\nv 1e400 0 0\n".to_vec()),
        ("z.stl", vec![0u8; 90]),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let _ = e.execute("plan.open", &json!({"path": path.to_string_lossy()}));
        let _ = e.execute("coverage.compute", &json!({}));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
