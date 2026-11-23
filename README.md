# RadioCraft (beta)

<img src="assets/icon.svg" alt="RadioCraft's logo: a router sending out rings of signal in the heatmap's colours, which turn weaker where they have passed the corner of a wall" width="128">

RadioCraft plans Wi-Fi coverage from the floor plans and models you already have. Open a DXF or
DWG plan from CADCraft, a SolidCraft design, or an OBJ or STL model. Tell it what each layer or
object is made of, place routers and access points, and see the predicted signal across the floor,
or across several floors.

![RadioCraft with CADCraft's example apartment: two routers on overlapping 5 GHz channels (36 at 80 MHz and 44), and the map of where a device hears the other one on its channel](docs/screenshot.png)

It is a beta. The numbers are a planning estimate from a simple indoor model, not a site survey;
see [What the numbers mean](#what-the-numbers-mean).

## What it does

- **Plans from other Craft apps.** A CADCraft drawing (DXF, or DWG through the `acadrust` crate
  as CADCraft reads it) is read by its layers (`A-WALL`, `A-GLAZ`, `A-DOOR`, `A-FURN` and the
  like); its named views can frame the map. A SolidCraft design (`.sc`) opens through SolidCraft's
  own `solidcraft-cli`, so SolidCraft must be installed (or point `RADIOCRAFT_SOLIDCRAFT_CLI` at
  its CLI). It and OBJ and STL models are cut at a height (1 m by default) and read by their
  objects, one per body. Blocks, arcs, bulged polylines and hatches are read; walls drawn as
  closed outlines keep their rooms open.
- **Viewpoints.** A model is seen from the top (a floor plan), the bottom, front, back, left or
  right, or from a view saved in the SolidCraft design, and cut at a depth toward you; routers
  stay where they are in the building when you change it.
- **The Craft header.** The same title bar, menus (in the macOS menu bar), toolbar, status bar
  and Home page as the other Craft apps, through craft-libs' `craft-window` and `craft-menu`. On
  Windows and Linux the app's bar is the title bar, with the desktop's window buttons in its
  order and look (Windows, GNOME's Adwaita, KDE's Breeze) and resizable edges; tiling desktops,
  and `RADIOCRAFT_TITLEBAR=system`, keep the system's title bar. `RADIOCRAFT_CAPTIONS=windows`
  (or `adwaita`, `breeze`, `generic`) shows a desktop's buttons on any platform, for checking.
- **Materials.** Each layer or object is concrete, drywall, low-E glass, interior glass, metal,
  a wood door, "not a wall", or "guess from thickness" (drywall below 140 mm, concrete from there).
  The first guess comes from the names (`glass`, `door`, `fridge`, `sofa`…); change any of them.
  A custom loss in dB for each band is allowed too.
- **Routers.** Place, drag and remove routers. Each has a height and a device: generic home
  routers, a ceiling access point and a mesh node are built in, and a device file built from FCC
  and ISED certification records adds real products.
- **Coverage.** A heatmap for 2.4, 5 or 6 GHz; the share of the floor at −60 dBm or better (fast)
  and −67 dBm or better (video calls), the weakest 5 % and the median.
- **Channels.** Give each router a channel and width per band; channels are checked against the
  band's channel plan. A bare number (`"5": 36`) is a 20 MHz channel; `{"channel": 36}` without
  a width is 20 MHz on 2.4 GHz and 80 MHz on 5 and 6 GHz.
- **Maps.** Besides the signal: the second-strongest signal (is there a router to roam to?), the
  serving router, how many routers on an overlapping channel are heard (at the 802.11 −82 dBm
  level), the estimated link rate (from the 802.11 receiver sensitivity table, with nothing else
  on the air), and signal outside the walls, out to 3 m past the plan's edge (leakage; spaces inside the
  plan that the walls don't enclose, such as balconies and wall cavities, count as outside).
- **Probe.** Hover anywhere: the three strongest routers with their signal and channel, the link
  rate, and the routers sharing the channel there.
- **Suggest a spot.** Place a router where it raises the floor at −67 dBm or better the most,
  given the routers already there; repeat for more.
- **Measurements.** Read measured points (CSV or JSON), see how far the prediction is from them,
  and calibrate: fit the path-loss exponent, a per-band offset and the losses of the walls enough
  points are measured through to them. Calibrate… shows the fit before it is used: each wall's
  loss before and after, and the error before, after and on points left out of the fit.
- **Reports.** One HTML page with every band's figures and maps, the routers, the walls and the
  settings; print it to PDF from a browser.
- **Projects.** Save the materials, routers, channels, measurements and settings as a `.rcraft`
  file beside the plan. Export the shown map as a PNG.
- **Several floors.** Stack floors: each has its own plan (a DXF, or the same model cut higher),
  an elevation, an offset to line plans up, and the slab under it (concrete unless you choose;
  "none" for an opening). A router on one floor reaches the others along the straight path
  through the building: the walls on each floor it passes, plus each slab once, the same both
  ways. Routers and measured points belong to a floor (a `floor` column in a measurements CSV);
  routers on other floors show faded. Floors stack in the top view; from a side, the shown floor
  stands alone.
- **Undo and redo.** Every edit (materials, routers, channels, settings, measurements, devices)
  can be undone and redone: Edit menu, ⌘Z and ⇧⌘Z, or the bar's buttons, named for what they do.
  A drag is one step; switching the band or map shown isn't an edit; opening a plan starts a new
  history. Scripts and the CLI use `edit.undo`, `edit.redo` and `edit.history`.
- **Everything is a command.** The window, the CLI and scripts all run the same commands.

## Build and run

Rust 1.90 or later.

```bash
cargo run --release -p radiocraft
```

```bash
cargo run --release -p radiocraft -- tests/fixtures/cadcraft-apartment.dxf
```

### Install on macOS

```bash
scripts/install-macos.sh
```

It builds a release and installs `RadioCraft.app` in `~/Applications` (set `RADIOCRAFT_APP_DIR`
for elsewhere); the command-line tool is inside it at
`RadioCraft.app/Contents/MacOS/radiocraft-cli`. To keep the installed app up to date while
developing, add local git hooks that reinstall it in the background after every commit, merge,
pull or branch checkout:

```bash
scripts/install-macos.sh --hooks
```

Progress goes to `~/Library/Logs/RadioCraft/install.log`, and a notification says when a build
is installed. Quit and reopen the app to run the new build.

The command-line tool runs the same engine without a window:

```bash
cargo run --release -p radiocraft-cli -- plan tests/fixtures/cadcraft-apartment.dxf --router 1200,1200,900 --device generic-router --material A-WALL=concrete --png coverage.png
```

```bash
cargo run --release -p radiocraft-cli -- script docs/examples/cadcraft-apartment.json
```

Measured points are a CSV with a header row:

```text
x,y,band,rssi_dbm,router
1000,800,5,-42.4,Living room
```

Positions in millimetres on the plan, the signal in dBm, and optionally the router it was
measured from (otherwise the strongest one). `docs/examples/survey-synthetic.csv` is made up (a
known model plus noise) to try it on the CADCraft example: calibrating finds an exponent near 3.

Calibrating wall losses: a material's loss in a band is fitted only when at least 5 points are
measured through it, and not all through it alike (that would be the offset's job); the others
keep their loss. Fitted losses are held toward their current values, so a few points move them
little, and stay within 0–60 dB. They are the project's own, from its measurements: the
materials' presets don't change, and Reset (or `settings.set {"loss_db": {}}`) puts them back.
The fit is linear least squares, counted again once with the fitted losses (where two walls
overlap, a new loss can change which counts). The error on points left out (each fifth of the
points predicted by a fit to the rest) says how it will do at places not measured; when it is
worse than the model as it was, RadioCraft says so.

`radiocraft-cli commands` lists every command and its parameters. A script is
`{"commands": [{"command": "...", "params": {...}}, ...]}`, run in order; the window takes the
same file with `--script` (and `--screenshot out.png` to save a picture of the window and quit).

### From SolidCraft

Export the design as OBJ (File → Export, OBJ). SolidCraft writes one object per body, in
millimetres with Z up, which is what RadioCraft expects. Open the `.obj`; set the slice height in
the top bar if 1 m doesn't cut through the walls. An STL has no object names, so it arrives as one
object; give it a material by hand.

### From CADCraft

Save the plan as DXF. Layer names choose the first materials; anything on a furniture,
dimension or annotation layer is left out.

## Device files

A device file is a JSON list:

```json
[{"id": "u7-pro", "name": "U7 Pro", "eirp_dbm": {"2.4": 25.0, "5": 29.0, "6": 26.5}}]
```

Load it with **Device file…** or `devices.load`. EIRP is the conducted power plus antenna gain,
as the certification records give it; values above the usual regulatory limits (30 dBm at 2.4
and 5 GHz, 27 dBm at 6 GHz) are capped unless `cap_eirp` is turned off.

## What the numbers mean

The signal at a point is

> EIRP − path loss at 1 m − 10 · n · log10(distance) − the loss of each wall on the way

with n = 2.2, the path loss at 1 m from the band's centre frequency, and the distance in 3D
between the router and a device held at 1 m. Each wall is counted once per crossing, exactly, from
the plan's outlines (not from the heatmap's grid), and where two materials overlap the larger loss
counts. Reflections, furniture, people, interference and the client's own radio aren't modelled;
real signal is often a few dB either side. Use it to compare placements, then check on site.

## Status

Known limits:

- Floors: slabs are uniform (no stairwells, shafts or openings in them unless the whole slab is
  "none"), and calibration fits walls' losses, not the slabs'.
- Where walls overlap the larger loss counts, so a window drawn over an unbroken wall outline
  counts as wall, and its glass can't be calibrated.

- A SolidCraft design opens through the installed SolidCraft. In the window SolidCraft exports it
  in the background (the status bar shows it, with Cancel); the CLI and scripts wait for it. An
  installed SolidCraft older than
  the build that saved the design may read it with warnings (they show in the plan's notes);
  `RADIOCRAFT_SOLIDCRAFT_CLI` can point at another build's `solidcraft-cli`.
- Saved SolidCraft views are read when they are Z-up; a DXF's named views are used when they
  look straight down in model space (twist ignored; paper-space views and layouts aren't offered).

Beta. Working: DXF, DWG, SolidCraft and OBJ/STL plans, viewpoints, several floors, materials,
routers, devices, channels, the three bands, six maps and their figures, the probe, spot
suggestions, measured points and calibration (wall losses included), undo and redo, HTML reports,
projects, PNG export, the CLI and scripts. Not yet: channel suggestions, antenna patterns,
openings in slabs, live scanning of networks, translations beyond English (strings go through
`i18n::tr` already).
See [docs/plan.md](docs/plan.md).

## Licence

MIT OR Apache-2.0, at your option. Third-party files are listed in
[ATTRIBUTION.md](ATTRIBUTION.md).
