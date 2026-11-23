# ArtCraft Beta signup: draft

Draft answers for the form at https://getartcraft.com/beta (fields: App name, Email, Repo,
Description, Website). Nothing here has been sent. Fill in the email when you submit, and clear
the name check at the end first.

---

**App name**

RadioCraft

**Email**

*(your contact address)*

**Repo**

https://github.com/DevonHope/RadioCraft

**Description** (the form's free text; about 150 words)

> RadioCraft plans Wi-Fi coverage from the drawings and models people already make in the Craft
> apps. Open a CADCraft DXF or DWG plan or a SolidCraft design (cut at a height, from any side or
> a saved view), give each layer or object a material, place routers and access points, and see
> the predicted signal on 2.4, 5 and 6 GHz, stacked over several floors. Maps show the signal,
> the second-strongest router, the serving router, same-channel routers heard, link rate and
> leakage outside the walls; it suggests where the next router does most good. Measured points
> calibrate the model, wall losses included, with the error on points left out shown before
> anything is applied. Every action is a command, run the same from the window, the CLI or a
> script, with undo. It uses the Craft header (craft-window, craft-menu) and reads files through
> craft-libs. Pure Rust and egui, MIT OR Apache-2.0, clean room. Working beta.

**Website**

*(leave empty, or https://github.com/DevonHope/RadioCraft#readme)*

---

## Notes before sending

- **Name check (blocks sending).** "NetCraft" was the first choice, but NETCRAFT is a live US
  trademark of Netcraft Ltd for software and computer services, so it fails the Craft naming
  standard (`standards/naming.md`, "unclear means no"). "RadioCraft" is unclear too: an
  ARRL-funded Minecraft radio mod and other software projects use it, and Radiocrafts AS makes RF
  radio modules. The maintainer chose to keep RadioCraft (2026-11-22); CoverageCraft and
  AirwaveCraft are the fallbacks. Before sending, run the trademark register search (USPTO,
  EUIPO, WIPO; classes 9 and 42), record it in `docs/feature-names.md`, and mention the other
  uses of the name in the form so the beta team can judge.
- **"Wi-Fi"** is a Wi-Fi Alliance trademark. The description uses it only to say what the app
  plans, as the naming standard allows for a technology the app works with. Feature labels say
  "signal", "coverage" and "router" instead.
- **Prior art in the Craft orgs.** A draft pull request in SolidCraft
  (tangency-labs/solidcraft#20164, still open as a draft) adds the same analysis inside
  SolidCraft. Say in the form or a follow-up whether that PR stays, shrinks to the generic
  pieces (RF materials, slicing) or closes, so the beta team doesn't see two competing versions.
- **Third-party code to mention.** DWG plans are read with `acadrust` (MPL-2.0, unmodified), as
  CADCraft reads them. egui brings in `self_cell`, licensed "Apache-2.0 OR GPL-2.0-only" (used
  under Apache-2.0), as in every egui app.
- **What isn't verified.** Windows and Linux builds compile and lint clean but haven't been run
  on those systems. Calibration has been tried only on made-up measurements.
- **Open questions to ask the beta team:**
  - whether a beta app should copy `standards/app-agents.md` into its `AGENTS.md` now or on
    acceptance;
  - the app id (`ai.storyteller.radiocraft`, as accepted apps use, or your own until then);
  - whether a network-scanning phase (live site surveys) is welcome, since it needs OS network
    access the other Craft apps don't use.
