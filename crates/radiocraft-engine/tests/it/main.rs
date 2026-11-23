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
