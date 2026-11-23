# Attribution

| File | Source | Licence | Version | Notes |
|---|---|---|---|---|
| `tests/fixtures/cadcraft-apartment.dxf` | storytold/cadcraft, `examples/apartment.dxf` (https://github.com/storytold/cadcraft) | Apache-2.0 | commit a70b9b02e4fb7f2c3b50902d33e8daf96d904053 | A floor plan drawn with CADCraft's commands, used unchanged as a test input |
| `crates/radiocraft-engine/src/model.rs`, `materials.rs` | Written by this project's author for SolidCraft (tangency-labs/solidcraft#20164) | MIT OR Apache-2.0 | 2026-11-22 | The coverage model and RF material presets, adapted |
| `docs/screenshot.png` | A screenshot of RadioCraft with the CADCraft fixture above open | MIT OR Apache-2.0 (the plan in it: Apache-2.0) | 2026-11-22 | Made by this project |
| `docs/examples/survey-synthetic.csv` | Made by this project: synthetic, from RadioCraft's own model of the CADCraft fixture (exponent 3.0, offsets +2 dB at 2.4 GHz and −4 dB at 5 GHz) plus ±2 dB noise; not a measurement | MIT OR Apache-2.0 | 2026-11-22 | For trying measured points and calibration |
| `docs/examples/cadcraft-apartment.json` | Made by this project: a command script for the CADCraft fixture | MIT OR Apache-2.0 | 2026-11-22 | |
| `assets/icon.vectorcraft`, `assets/icon.svg`, `assets/icon-1024.png` | Made by this project: an original logo drawn in VectorCraft (storytold/vectorcraft, 0.24.1) through its control channel; the SVG and PNG are VectorCraft's exports of the `.vectorcraft` source | MIT OR Apache-2.0 | 2026-11-22 | Edit the `.vectorcraft` file in VectorCraft and export both again; don't edit the exports by hand |
| `tests/fixtures/solidcraft-demo.sc` | Made by this project: a made-up flat (walls, partitions, a core, a door, a fridge) built in SolidCraft for testing; no real place | MIT OR Apache-2.0 | 2026-11-22 | Opened through SolidCraft's own `solidcraft-cli` |
| `acadrust` 0.6.8 (Cargo dependency, used unmodified, for reading DWG) | https://github.com/hakanaktt/acadrust, crates.io `acadrust` 0.6.8 | MPL-2.0 | 0.6.8 | Its source is on crates.io and at that repository; any RadioCraft package must keep its licence and notice files |
