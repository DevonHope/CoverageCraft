//! CoverageCraft's engine on real files: CADCraft's apartment plan, and a model as SolidCraft exports it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // Tests fail by panicking.

use coveragecraft_engine::Engine;
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
    let dir = std::env::temp_dir().join(format!("coveragecraft-it-{}", std::process::id()));
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
    let dir = std::env::temp_dir().join(format!("coveragecraft-project-{}", std::process::id()));
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
    let project = dir.join("home.coveragecraft");
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
    let dir = std::env::temp_dir().join(format!("coveragecraft-report-{}", std::process::id()));
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
    assert_eq!(e.settings().view, coveragecraft_engine::radio::View::Rate);
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
    let dir = std::env::temp_dir().join(format!("coveragecraft-survey-{}", std::process::id()));
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
    let project = dir.join("p.coveragecraft");
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

/// A model seen from the front is cut across its depth, and a router stays where it is in the
/// building when the viewpoint changes.
#[test]
fn a_model_can_be_seen_from_the_front() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-front-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("room.obj");
    std::fs::write(&path, solidcraft_obj()).unwrap();
    let mut e = Engine::new();
    let info = run(
        &mut e,
        "plan.open",
        json!({"path": path.to_string_lossy(), "slice_height": 1000}),
    );
    assert_eq!(info["viewpoint"], "top");
    assert_eq!(
        info["viewpoints"].as_array().map(Vec::len),
        Some(6),
        "{info}"
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [1000, 1500], "height": 1200, "eirp_dbm": 20}),
    );
    // From the front the map is x across and z up, and the cut is 1.5 m into the room
    // (toward the viewer is −y, so the cut's depth is −1500).
    run(
        &mut e,
        "settings.set",
        json!({"viewpoint": "front", "slice_height": -1500}),
    );
    let front = run(&mut e, "plan.info", json!({}));
    assert_eq!(front["viewpoint"], "front");
    // The side walls are cut; the front and back walls lie in front of and behind the cut.
    assert_eq!(source(&front, "Wall W")["shapes"], 1, "{front}");
    assert!(
        front["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["name"] != "Wall S"),
        "{front}"
    );
    let routers = run(&mut e, "router.list", json!({}));
    assert_eq!(routers[0]["at"], json!([1000.0, 1200.0]), "{routers}");
    assert_eq!(routers[0]["height"], -1500.0);
    // A project keeps the viewpoint: it opens seen from the front again, the router in place.
    let project = dir.join("front.coveragecraft");
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
    assert_eq!(
        run(&mut again, "plan.info", json!({}))["viewpoint"],
        "front"
    );
    assert_eq!(
        run(&mut again, "router.list", json!({}))[0]["at"],
        json!([1000.0, 1200.0])
    );
    // The map's points lie on the cut, as the router does: 2 m straight above it, with no wall
    // between, the signal is free space over 2 m: 20 − 47.257 − 22·log10(2) = −33.88 dBm.
    let above = run(&mut e, "coverage.at", json!({"at": [1000, 3200]}));
    assert_eq!(above["bands"]["5"]["rssi_dbm"], -33.9, "{above}");
    // And back.
    run(
        &mut e,
        "settings.set",
        json!({"viewpoint": "top", "slice_height": 1000}),
    );
    let routers = run(&mut e, "router.list", json!({}));
    assert_eq!(routers[0]["at"], json!([1000.0, 1500.0]), "{routers}");
    assert!(
        e.execute("settings.set", &json!({"viewpoint": "sideways"}))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A drawing's named view frames part of it: the map covers that window.
#[test]
fn a_named_view_frames_the_map() {
    let dxf = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nVIEW\n70\n1\n\
0\nVIEW\n2\nLeft room\n70\n0\n40\n4000\n41\n3000\n10\n1500\n20\n2000\n11\n0\n21\n0\n31\n1\n12\n0\n22\n0\n32\n0\n50\n0\n\
0\nENDTAB\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLWPOLYLINE\n8\nA-WALL\n90\n4\n70\n1\n10\n0\n20\n0\n10\n8000\n20\n0\n10\n8000\n20\n4000\n10\n0\n20\n4000\n\
0\nLWPOLYLINE\n8\nA-WALL\n90\n4\n70\n1\n10\n150\n20\n150\n10\n7850\n20\n150\n10\n7850\n20\n3850\n10\n150\n20\n3850\n\
0\nENDSEC\n0\nEOF\n";
    let dir = std::env::temp_dir().join(format!("coveragecraft-view-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plan.dxf");
    std::fs::write(&path, dxf).unwrap();
    let mut e = Engine::new();
    let info = run(&mut e, "plan.open", json!({"path": path.to_string_lossy()}));
    let ids: Vec<&str> = info["viewpoints"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert_eq!(ids, vec!["top", "view:Left room"], "{info}");
    assert_eq!(
        info["viewpoints"][1]["window"],
        json!([0.0, 0.0, 3000.0, 4000.0])
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [1500, 2000], "height": 2400}),
    );
    let whole = run(&mut e, "coverage.compute", json!({}));
    run(
        &mut e,
        "settings.set",
        json!({"viewpoint": "view:Left room"}),
    );
    let framed = run(&mut e, "coverage.compute", json!({}));
    // The map covers the 3 m × 4 m window, not the 8 m × 4 m drawing.
    let cols = |c: &Value| c["grid"]["columns"].as_u64().unwrap();
    assert!(cols(&framed) < cols(&whole), "{whole} {framed}");
    assert!(framed["floor_m2"].as_f64().unwrap() < whole["floor_m2"].as_f64().unwrap());
    // A drawing has no side views.
    assert!(
        e.execute("settings.set", &json!({"viewpoint": "front"}))
            .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A SolidCraft design opens through SolidCraft's own exporter, with its saved views; without
/// SolidCraft installed the test says so and passes.
#[test]
fn a_solidcraft_design_opens() {
    if coveragecraft_engine::sc::find_cli().is_none() {
        eprintln!("SolidCraft isn't installed here (no solidcraft-cli): skipped");
        return;
    }
    let mut e = Engine::new();
    let info = run(
        &mut e,
        "plan.open",
        json!({"path": fixture("solidcraft-demo.sc"), "slice_height": 1000}),
    );
    assert_eq!(source(&info, "Wall S")["shapes"], 1, "{info}");
    assert_eq!(source(&info, "Partition A")["shapes"], 1, "{info}");
    run(
        &mut e,
        "router.add",
        json!({"at": [2000, 2000], "height": 2400, "device": "generic-ceiling-ap"}),
    );
    let c = run(&mut e, "coverage.compute", json!({}));
    assert_eq!(c["floor"], "enclosed", "{c}");
    // A view saved in the design (by SolidCraft itself, in a temporary copy) is offered, and
    // picking it cuts the design along it: this one looks from the street side, like Front.
    let dir = std::env::temp_dir().join(format!("coveragecraft-scview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("save-view.json");
    std::fs::write(
        &script,
        json!({"commands": [{"command": "view.save", "params": {"name": "Street side",
            "camera": {"target": [5000, 3500, 1200], "yaw": 0.0, "pitch": 0.0, "distance": 20000}}}]})
        .to_string(),
    )
    .unwrap();
    let copy = dir.join("with-view.sc");
    let status = std::process::Command::new(coveragecraft_engine::sc::find_cli().unwrap())
        .arg("run")
        .arg(&script)
        .arg("--in")
        .arg(fixture("solidcraft-demo.sc"))
        .arg("--save")
        .arg(&copy)
        .arg("--quiet")
        .env("SOLIDCRAFT_SETTINGS_DIR", dir.join("settings"))
        .env("SOLIDCRAFT_RECOVERY_DIR", dir.join("recovery"))
        .env("SOLIDCRAFT_CONTROL_DIR", dir.join("control"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let mut v = Engine::new();
    let info = run(&mut v, "plan.open", json!({"path": copy.to_string_lossy()}));
    let ids: Vec<&str> = info["viewpoints"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x["id"].as_str())
        .collect();
    assert!(ids.contains(&"view:Street side"), "{ids:?}");
    run(
        &mut v,
        "settings.set",
        json!({"viewpoint": "view:Street side", "slice_height": -3500}),
    );
    let side = run(&mut v, "plan.info", json!({}));
    assert_eq!(side["viewpoint"], "view:Street side");
    assert_eq!(source(&side, "Wall W")["shapes"], 1, "{side}");
    let _ = std::fs::remove_dir_all(&dir);
    // Seen from the front, the same design opens from the cache (no second export).
    let started = std::time::Instant::now();
    run(
        &mut e,
        "settings.set",
        json!({"viewpoint": "front", "slice_height": -3500}),
    );
    assert!(started.elapsed().as_secs_f64() < 5.0);
    assert_eq!(run(&mut e, "plan.info", json!({}))["viewpoint"], "front");
}

/// A DWG opens as the same plan as its DXF: the example apartment, written as DWG by acadrust.
#[test]
fn a_dwg_plan_opens() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-dwg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let doc = acadrust::DxfReader::from_file(fixture("cadcraft-apartment.dxf"))
        .unwrap()
        .read()
        .unwrap();
    let path = dir.join("apartment.dwg");
    std::fs::write(&path, acadrust::DwgWriter::write_to_vec(&doc).unwrap()).unwrap();
    let mut e = Engine::new();
    let info = run(&mut e, "plan.open", json!({"path": path.to_string_lossy()}));
    assert_eq!(
        source(&info, "A-GLAZ")["material"],
        "interior-glass",
        "{info}"
    );
    let b = info["bounds"].as_array().unwrap();
    assert!(
        (b[2].as_f64().unwrap() - b[0].as_f64().unwrap() - 12000.0).abs() < 1.0,
        "{info}"
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [6000, 4000], "height": 2400}),
    );
    assert!(
        run(&mut e, "coverage.compute", json!({}))["floor_m2"]
            .as_f64()
            .unwrap()
            > 50.0
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Edits undo and redo, a drag is one step, the band and map shown aren't edits, and opening a
/// plan starts a new history.
#[test]
fn undo_and_redo() {
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    assert!(
        e.execute("edit.undo", &json!({})).is_err(),
        "nothing to undo after opening"
    );
    let plain = run(&mut e, "coverage.compute", json!({}));
    run(
        &mut e,
        "router.add",
        json!({"at": [1000, 1000], "height": 2400}),
    );
    // A drag: many moves, one step.
    for x in [1500, 2000, 2500, 3000] {
        run(&mut e, "router.move", json!({"id": 1, "at": [x, 1000]}));
    }
    run(
        &mut e,
        "material.set",
        json!({"source": "A-WALL", "material": "concrete"}),
    );
    // Showing another band or map isn't an edit.
    run(
        &mut e,
        "settings.set",
        json!({"band": "2.4", "view": "rate"}),
    );
    let h = run(&mut e, "edit.history", json!({}));
    assert_eq!(
        h["undo"],
        json!(["Add Router", "Move Router", "Change Material"]),
        "{h}"
    );
    let moved_concrete = run(&mut e, "coverage.compute", json!({}));
    assert_eq!(
        run(&mut e, "edit.undo", json!({}))["undone"],
        "Change Material"
    );
    assert_eq!(
        run(&mut e, "plan.info", json!({}))["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "A-WALL")
            .unwrap()["material"],
        "auto"
    );
    // The band and map shown stay as they are.
    assert_eq!(e.settings().band.name(), "2.4");
    run(&mut e, "edit.undo", json!({}));
    assert_eq!(
        run(&mut e, "router.list", json!({}))[0]["at"],
        json!([1000.0, 1000.0])
    );
    run(&mut e, "edit.undo", json!({}));
    assert_eq!(run(&mut e, "router.list", json!({})), json!([]));
    run(
        &mut e,
        "settings.set",
        json!({"band": "5", "view": "signal"}),
    );
    assert_eq!(
        run(&mut e, "coverage.compute", json!({}))["routers"],
        plain["routers"]
    );
    // Redo all three: back where we were.
    for _ in 0..3 {
        run(&mut e, "edit.redo", json!({}));
    }
    run(
        &mut e,
        "settings.set",
        json!({"band": "2.4", "view": "rate"}),
    );
    assert_eq!(run(&mut e, "coverage.compute", json!({})), moved_concrete);
    assert!(e.execute("edit.redo", &json!({})).is_err());
    // A new edit drops what could be redone.
    run(&mut e, "edit.undo", json!({}));
    run(&mut e, "router.add", json!({"at": [5000, 5000]}));
    assert_eq!(run(&mut e, "edit.history", json!({}))["redo"], json!([]));
    // An edit.checkpoint between two drags keeps them apart.
    run(&mut e, "router.move", json!({"id": 1, "at": [1, 1]}));
    run(&mut e, "edit.checkpoint", json!({}));
    run(&mut e, "router.move", json!({"id": 1, "at": [2, 2]}));
    let n = run(&mut e, "edit.history", json!({}))["undo"]
        .as_array()
        .unwrap()
        .len();
    run(&mut e, "edit.undo", json!({}));
    assert_eq!(
        run(&mut e, "router.list", json!({}))[0]["at"],
        json!([1.0, 1.0])
    );
    assert_eq!(
        run(&mut e, "edit.history", json!({}))["undo"]
            .as_array()
            .unwrap()
            .len(),
        n - 1
    );
    // Opening a plan starts again.
    run(
        &mut e,
        "plan.open",
        json!({"path": fixture("cadcraft-apartment.dxf")}),
    );
    assert_eq!(
        run(&mut e, "edit.history", json!({})),
        json!({"undo": [], "redo": []})
    );
}

/// Undoing a change of viewpoint cuts the model the old way again, routers back in place.
#[test]
fn undoing_a_viewpoint_cuts_the_model_again() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-undo-view-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("room.obj");
    std::fs::write(&path, solidcraft_obj()).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": path.to_string_lossy(), "slice_height": 1000}),
    );
    run(
        &mut e,
        "router.add",
        json!({"at": [1000, 1500], "height": 1200}),
    );
    let top = run(&mut e, "plan.info", json!({}));
    run(
        &mut e,
        "settings.set",
        json!({"viewpoint": "front", "slice_height": -1500}),
    );
    assert_eq!(
        run(&mut e, "edit.undo", json!({}))["undone"],
        "Change Viewpoint"
    );
    assert_eq!(run(&mut e, "plan.info", json!({})), top);
    assert_eq!(
        run(&mut e, "router.list", json!({}))[0]["at"],
        json!([1000.0, 1500.0])
    );
    run(&mut e, "edit.redo", json!({}));
    assert_eq!(run(&mut e, "plan.info", json!({}))["viewpoint"], "front");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A design exported on another thread (as the window does) opens from that export, also
/// through a project; a cancelled export stops at once.
#[test]
fn a_design_exported_in_the_background_opens_from_it() {
    use coveragecraft_engine::sc;
    if sc::find_cli().is_none() {
        eprintln!("SolidCraft isn't installed here (no solidcraft-cli): skipped");
        return;
    }
    let design = fixture("solidcraft-demo.sc");
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let started = std::time::Instant::now();
    assert_eq!(
        sc::convert_with(std::path::Path::new(&design), &cancel)
            .err()
            .as_deref(),
        Some(sc::CANCELLED)
    );
    assert!(started.elapsed().as_secs_f64() < 5.0);
    let path = design.clone();
    let mut converted = std::thread::spawn(move || sc::convert(std::path::Path::new(&path)))
        .join()
        .unwrap()
        .unwrap();
    // Marked, to tell this export from a new one.
    converted.notes.push("exported in the background".into());
    let mut e = Engine::new();
    e.prime_sc(&design, converted).unwrap();
    let info = run(&mut e, "plan.open", json!({"path": design}));
    assert!(
        info["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "exported in the background"),
        "{info}"
    );
    let dir = std::env::temp_dir().join(format!("coveragecraft-bg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.join("p.coveragecraft");
    run(
        &mut e,
        "project.save",
        json!({"path": project.to_string_lossy()}),
    );
    run(
        &mut e,
        "project.open",
        json!({"path": project.to_string_lossy()}),
    );
    let again = run(&mut e, "plan.info", json!({}));
    assert!(
        again["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "exported in the background"),
        "{again}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A square room (outer walls) in a DXF, drawn with its corner at `x0`.
fn room_dxf(x0: f64) -> String {
    let ring = |a: f64, b: f64| {
        format!(
            "0\nLWPOLYLINE\n8\nA-WALL\n90\n4\n70\n1\n10\n{}\n20\n{}\n10\n{}\n20\n{}\n10\n{}\n20\n{}\n10\n{}\n20\n{}\n",
            x0 + a,
            a,
            x0 + b,
            a,
            x0 + b,
            b,
            x0 + a,
            b
        )
    };
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{}{}0\nENDSEC\n0\nEOF\n",
        ring(0.0, 6000.0),
        ring(150.0, 5850.0)
    )
}

/// Floors: a router a floor up is heard through the slab; two floors up through two; floors
/// line up by their offsets; a project keeps them; adding, changing and removing undo; showing
/// another floor isn't an edit; and bad floor commands are refused.
#[test]
fn floors_stack() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-floors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ground, upper) = (dir.join("ground.dxf"), dir.join("upper.dxf"));
    std::fs::write(&ground, room_dxf(0.0)).unwrap();
    // The upper floor's plan is drawn 20 m to the right: its offset brings it over the ground's.
    std::fs::write(&upper, room_dxf(20000.0)).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": ground.to_string_lossy()}),
    );
    assert_eq!(
        run(&mut e, "floor.list", json!({}))["floors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let added = run(
        &mut e,
        "floor.add",
        json!({"name": "Upstairs", "offset": [-20000, 0]}),
    );
    assert_eq!(added["index"], 1);
    let info = run(
        &mut e,
        "plan.open",
        json!({"path": upper.to_string_lossy()}),
    );
    assert_eq!(info["bounds"], json!([0.0, 0.0, 6000.0, 6000.0]), "{info}");
    assert_eq!(info["floor"], 1);
    // A router upstairs, 1 m above its floor (4 m up); the probe downstairs straight below, at
    // 1 m: 3 m apart, one concrete slab: 20 − 47.257 − 22·log10(3) − 20 = −57.75 dBm.
    run(
        &mut e,
        "router.add",
        json!({"at": [3000, 3000], "height": 1000, "eirp_dbm": 20, "name": "Up"}),
    );
    run(&mut e, "floor.select", json!({"index": 0}));
    let below = run(&mut e, "coverage.at", json!({"at": [3000, 3000]}));
    assert_eq!(below["bands"]["5"]["rssi_dbm"], -57.8, "{below}");
    assert_eq!(below["bands"]["5"]["floor"], 1);
    // Showing another floor isn't an edit.
    let h = run(&mut e, "edit.history", json!({}));
    assert_eq!(
        h["undo"],
        json!(["Add Floor", "Open Plan", "Add Router"]),
        "{h}"
    );
    // Two floors up: 6 m and two slabs: 20 − 47.257 − 22·log10(6) − 40 = −84.38 dBm.
    run(&mut e, "floor.add", json!({"name": "Roof"}));
    run(&mut e, "router.set", json!({"id": 1, "floor": 2}));
    run(&mut e, "floor.select", json!({"index": 0}));
    assert_eq!(
        run(&mut e, "coverage.at", json!({"at": [3000, 3000]}))["bands"]["5"]["rssi_dbm"],
        -84.4
    );
    // A slab of "not a wall" (an atrium) costs nothing: 20 − 47.257 − 17.12 − 20 (the upper one).
    run(&mut e, "floor.set", json!({"index": 2, "slab": "ignore"}));
    assert_eq!(
        run(&mut e, "coverage.at", json!({"at": [3000, 3000]}))["bands"]["5"]["rssi_dbm"],
        -64.4
    );
    // Undo the slab, the router's move and the roof.
    run(&mut e, "edit.undo", json!({}));
    assert_eq!(
        run(&mut e, "coverage.at", json!({"at": [3000, 3000]}))["bands"]["5"]["rssi_dbm"],
        -84.4
    );
    run(&mut e, "edit.undo", json!({}));
    run(&mut e, "edit.undo", json!({}));
    assert_eq!(
        run(&mut e, "floor.list", json!({}))["floors"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        run(&mut e, "coverage.at", json!({"at": [3000, 3000]}))["bands"]["5"]["rssi_dbm"],
        -57.8
    );
    // A project keeps the floors, their offsets, the router's floor and the floor shown.
    let project = dir.join("house.coveragecraft");
    run(
        &mut e,
        "project.save",
        json!({"path": project.to_string_lossy()}),
    );
    let text = std::fs::read_to_string(&project).unwrap();
    assert!(text.contains("coveragecraft-project/2"));
    let mut again = Engine::new();
    run(
        &mut again,
        "project.open",
        json!({"path": project.to_string_lossy()}),
    );
    let floors = run(&mut again, "floor.list", json!({}));
    assert_eq!(floors["floors"][1]["name"], "Upstairs");
    assert_eq!(floors["floors"][1]["offset"], json!([-20000.0, 0.0]));
    assert_eq!(floors["active"], 0);
    assert_eq!(
        run(&mut again, "coverage.at", json!({"at": [3000, 3000]}))["bands"]["5"]["rssi_dbm"],
        -57.8
    );
    // Removing a floor takes its routers; undo brings both back.
    run(&mut again, "floor.remove", json!({"index": 1}));
    assert_eq!(run(&mut again, "router.list", json!({})), json!([]));
    run(&mut again, "edit.undo", json!({}));
    assert_eq!(run(&mut again, "router.list", json!({}))[0]["floor"], 1);
    // Refused.
    for (command, params) in [
        ("floor.select", json!({"index": 9})),
        ("floor.remove", json!({})),
        ("floor.set", json!({"index": 5, "name": "x"})),
        ("floor.set", json!({"elevation": 1e300})),
        ("floor.set", json!({"offset": ["a", 1]})),
        ("floor.set", json!({"slab": "auto"})),
        ("floor.set", json!({"slab": "brick"})),
        ("router.add", json!({"at": [0, 0], "floor": 7})),
        (
            "survey.add",
            json!({"at": [0, 0], "band": "5", "rssi_dbm": -50, "floor": 7}),
        ),
    ] {
        assert!(
            again.execute(command, &params).is_err(),
            "{command} {params}"
        );
    }
    run(&mut again, "floor.remove", json!({"index": 1}));
    assert!(
        again.execute("floor.remove", &json!({"index": 0})).is_err(),
        "the last floor stays"
    );
    for _ in 0..31 {
        run(&mut again, "floor.add", json!({}));
    }
    assert!(
        again.execute("floor.add", &json!({})).is_err(),
        "32 floors at most"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A model seen from a side shows the shown floor alone, and says so.
#[test]
fn floors_stack_only_from_the_top() {
    let dir =
        std::env::temp_dir().join(format!("coveragecraft-floors-side-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("room.obj");
    std::fs::write(&path, solidcraft_obj()).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": path.to_string_lossy(), "slice_height": 1000}),
    );
    run(&mut e, "floor.add", json!({}));
    // The same model on the floor above is cut 3 m higher: above its walls, so nothing to see.
    assert!(
        e.execute("plan.open", &json!({"path": path.to_string_lossy()}))
            .is_ok()
    );
    assert_eq!(e.settings().slice_height, 4000.0);
    run(
        &mut e,
        "router.add",
        json!({"at": [1000, 1500], "height": 1000, "eirp_dbm": 20}),
    );
    run(&mut e, "floor.select", json!({"index": 0}));
    assert!(run(&mut e, "coverage.at", json!({"at": [1000, 1500]}))["bands"]["5"].is_object());
    run(
        &mut e,
        "settings.set",
        json!({"viewpoint": "front", "slice_height": -1500}),
    );
    let info = run(&mut e, "plan.info", json!({}));
    assert!(
        info["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("floors stack")),
        "{info}"
    );
    // The router upstairs doesn't send in the side view.
    assert!(
        run(&mut e, "coverage.at", json!({"at": [1000, 1000]}))["bands"]
            .get("5")
            .is_none()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A project saved before channels and maps existed opens, with them unset; it was saved when
/// the app was called RadioCraft (format 1, `.rcraft`).
#[test]
fn an_older_project_opens() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-old-{}", std::process::id()));
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
    assert_eq!(e.settings().view, coveragecraft_engine::radio::View::Signal);
    assert!(run(&mut e, "coverage.compute", json!({}))["pct_at_least_67"].is_number());
    // Format 2 under the old name opens too; a format nobody wrote is refused.
    let mut v2 = old.clone();
    v2["format"] = json!("radiocraft-project/2");
    std::fs::write(&path, v2.to_string()).unwrap();
    run(
        &mut e,
        "project.open",
        json!({"path": path.to_string_lossy()}),
    );
    v2["format"] = json!("coveragecraft-project/1");
    std::fs::write(&path, v2.to_string()).unwrap();
    assert!(
        e.execute("project.open", &json!({"path": path.to_string_lossy()}))
            .is_err()
    );
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
    let dir = std::env::temp_dir().join(format!("coveragecraft-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // None of these may crash; the DWG ones must also be refused.
    for (name, bytes) in [
        ("empty.dwg", Vec::new()),
        ("junk.dwg", b"AC1032 this is not a drawing at all".to_vec()),
        (
            "x.dxf",
            b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\nnan\n".to_vec(),
        ),
        ("y.obj", b"f 1 2 3\nv 1e400 0 0\n".to_vec()),
        ("z.stl", vec![0u8; 90]),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        let opened = e.execute("plan.open", &json!({"path": path.to_string_lossy()}));
        if name.ends_with(".dwg") {
            assert!(opened.is_err(), "{name}");
        }
        let _ = e.execute("coverage.compute", &json!({}));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Calibrated wall losses replace the presets' own on every floor, guessed walls too; undo puts
/// them back, a project keeps them, and bad values are refused.
#[test]
fn calibrated_wall_losses_apply_on_every_floor() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-losses-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (ground, upper) = (dir.join("ground.dxf"), dir.join("upper.dxf"));
    std::fs::write(&ground, room_dxf(0.0)).unwrap();
    std::fs::write(&upper, room_dxf(0.0)).unwrap();
    let mut e = Engine::new();
    run(
        &mut e,
        "plan.open",
        json!({"path": ground.to_string_lossy()}),
    );
    run(&mut e, "floor.add", json!({"name": "Upstairs"}));
    run(
        &mut e,
        "plan.open",
        json!({"path": upper.to_string_lossy()}),
    );
    // Upstairs, outside its room, high: the path down to the ground room's middle crosses the
    // upper room's (guessed) wall, then the slab, and no wall downstairs.
    run(
        &mut e,
        "router.add",
        json!({"at": [8000, 3000], "height": 2900, "eirp_dbm": 20, "name": "Up"}),
    );
    run(&mut e, "floor.select", json!({"index": 0}));
    let probe = |e: &mut Engine| {
        run(e, "coverage.at", json!({"at": [3000, 3000]}))["bands"]["5"]["rssi_dbm"]
            .as_f64()
            .unwrap()
    };
    let set = |e: &mut Engine, db: f64| {
        run(
            e,
            "settings.set",
            json!({"loss_db": {"drywall": {"5": db}, "concrete": {"5": db}}}),
        )
    };
    let preset = probe(&mut e);
    let s = set(&mut e, 30.0);
    assert_eq!(s["loss_db"]["concrete"], json!({"5": 30.0}), "{s}");
    let at30 = probe(&mut e);
    set(&mut e, 10.0);
    let at10 = probe(&mut e);
    assert!((at10 - at30 - 20.0).abs() < 0.11, "{at10} vs {at30}");
    assert_ne!(preset, at10);
    run(&mut e, "edit.undo", json!({}));
    assert_eq!(probe(&mut e), at30);
    // Kept in the project.
    let project = dir.join("p.coveragecraft");
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
    assert_eq!(again.settings().loss_db, e.settings().loss_db);
    run(&mut again, "floor.select", json!({"index": 0}));
    assert_eq!(probe(&mut again), at30);
    // One preset cleared, then all; the presets' own losses again.
    let s = run(
        &mut e,
        "settings.set",
        json!({"loss_db": {"drywall": null}}),
    );
    assert!(s["loss_db"].get("drywall").is_none(), "{s}");
    run(&mut e, "settings.set", json!({"loss_db": {}}));
    assert!(e.settings().loss_db.is_empty());
    assert_eq!(probe(&mut e), preset);
    // Refused, changing nothing.
    for bad in [
        json!({"loss_db": {"unobtainium": {"5": 3}}}),
        json!({"loss_db": {"concrete": {"5": -1}}}),
        json!({"loss_db": {"concrete": {"5": 101}}}),
        json!({"loss_db": {"concrete": {"7": 3}}}),
        json!({"loss_db": {"concrete": {"5": "x"}}}),
        json!({"loss_db": {"concrete": 3}}),
        json!({"loss_db": [1]}),
    ] {
        assert!(e.execute("settings.set", &bad).is_err(), "{bad}");
    }
    assert!(e.settings().loss_db.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wall losses come back from points made by a known model: concrete 28 dB at 5 GHz (its preset
/// says 20), with the exponent and offset, and the doors' loss stays where it was right; walls:
/// false fits only the exponent and offsets; with noise the held-out error is no better than the
/// fitted one; applying is one undoable step.
#[test]
fn calibration_finds_wall_losses() {
    let dir = std::env::temp_dir().join(format!("coveragecraft-walls-{}", std::process::id()));
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
        for (name, at) in [("Hall", [2500, 2500]), ("Den", [9000, 6000])] {
            run(
                e,
                "router.add",
                json!({"at": at, "height": 2400, "eirp_dbm": 20, "name": name}),
            );
        }
    };
    let mut truth = Engine::new();
    setup(&mut truth);
    run(
        &mut truth,
        "settings.set",
        json!({"exponent": 2.8, "offset_db": {"5": -3},
               "loss_db": {"concrete": {"5": 28}}}),
    );
    let mut survey = |noise: f64| {
        let mut csv = String::from("x,y,band,rssi_dbm,router\n");
        let mut k = 0u32;
        // Indoors, and a row outside the south windows (seen through the glass).
        let spots: Vec<[i32; 2]> = (500..12500)
            .step_by(700)
            .flat_map(|x| (500..8500).step_by(700).map(move |y| [x, y]))
            .chain((1000..11000).step_by(250).map(|x| [x, -800]))
            .collect();
        for [x, y] in spots {
            {
                for r in ["Hall", "Den"] {
                    let at = run(&mut truth, "coverage.at", json!({"at": [x, y]}));
                    let Some(v) = at["bands"]["5"]["routers"]
                        .as_array()
                        .and_then(|a| a.iter().find(|s| s["name"] == r))
                        .and_then(|s| s["rssi_dbm"].as_f64())
                    else {
                        continue;
                    };
                    // A meter hears no further down.
                    if v < -95.0 {
                        continue;
                    }
                    // A fixed pseudo-random spread, ±noise.
                    k = k.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    let n = (f64::from(k % 1000) / 500.0 - 1.0) * noise;
                    csv += &format!("{x},{y},5,{},{r}\n", v + n);
                }
            }
        }
        csv
    };
    let open = |csv: &str, name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, csv).unwrap();
        let mut e = Engine::new();
        setup(&mut e);
        run(
            &mut e,
            "survey.import",
            json!({"path": path.to_string_lossy()}),
        );
        e
    };
    let mut e = open(&survey(0.0), "exact.csv");
    assert!(e.survey().len() > 200, "{}", e.survey().len());
    let history = run(&mut e, "edit.history", json!({}));
    let fit = run(&mut e, "survey.calibrate", json!({}));
    let wall = |fit: &Value, m: &str| {
        fit["walls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["material"] == m && w["band"] == "5")
            .cloned()
            .unwrap_or_else(|| panic!("no {m}: {fit}"))
    };
    let concrete = wall(&fit, "concrete");
    assert_eq!(concrete["status"], "fitted", "{fit}");
    assert!(
        (concrete["fitted_db"].as_f64().unwrap() - 28.0).abs() < 1.0,
        "{fit}"
    );
    assert!(
        (fit["exponent"].as_f64().unwrap() - 2.8).abs() < 0.1,
        "{fit}"
    );
    assert!(
        (fit["offset_db"]["5"].as_f64().unwrap() + 3.0).abs() < 1.0,
        "{fit}"
    );
    for w in fit["walls"].as_array().unwrap() {
        if w["material"] != "concrete" {
            let (f, p) = (
                w["fitted_db"].as_f64().unwrap(),
                w["prior_db"].as_f64().unwrap(),
            );
            assert!((f - p).abs() < 0.2, "{fit}");
        }
    }
    assert!(fit["after"]["5"]["rms_db"].as_f64().unwrap() < 0.5, "{fit}");
    assert!(fit["note"].is_null(), "{fit}");
    // Without apply, nothing changes.
    assert!(e.settings().loss_db.is_empty());
    // Walls off: no loss moves.
    let plain = run(&mut e, "survey.calibrate", json!({"walls": false}));
    assert!(
        plain["walls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["status"] == "kept"),
        "{plain}"
    );
    // Looking at a fit isn't an edit.
    assert_eq!(run(&mut e, "edit.history", json!({})), history);
    // Applying is one step, undone in one.
    run(&mut e, "survey.calibrate", json!({"apply": true}));
    let got = e.settings().loss_db.get("concrete").copied().unwrap();
    assert!((got[1].unwrap() - 28.0).abs() < 1.0, "{got:?}");
    assert!(
        run(&mut e, "survey.compare", json!({}))["bands"]["5"]["rms_db"]
            .as_f64()
            .unwrap()
            < 0.5
    );
    run(&mut e, "edit.undo", json!({}));
    assert!(e.settings().loss_db.is_empty());
    assert_eq!(e.settings().exponent, 2.2);
    // With noise the error on points the fit didn't see is the larger.
    let mut noisy = open(&survey(6.0), "noisy.csv");
    let fit = run(&mut noisy, "survey.calibrate", json!({}));
    let (held, after) = (
        fit["held_out"]["5"]["rms_db"].as_f64().unwrap(),
        fit["after"]["5"]["rms_db"].as_f64().unwrap(),
    );
    assert!(held >= after, "{fit}");
    assert!(
        (wall(&fit, "concrete")["fitted_db"].as_f64().unwrap() - 28.0).abs() < 2.0,
        "{fit}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
