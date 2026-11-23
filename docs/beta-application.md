# ArtCraft Beta signup: draft

Draft answers for the form at https://getartcraft.com/beta (fields: App name, Email, Repo,
Description, Website). Nothing here has been sent. Fill in the email and repository when you
submit; check the name note at the end first.

---

**App name**

RadioCraft

**Email**

*(your contact address)*

**Repo**

*(the public repository, once it is published: for example `https://github.com/<you>/radiocraft`)*

**Description** (the form's free text; about 120 words)

> RadioCraft plans Wi-Fi coverage from the floor plans and models people already draw. Open a DXF
> plan from CADCraft or an OBJ model exported from SolidCraft. Give each layer or object a material
> (concrete, drywall, glass, metal, wood), place routers and access points, and see the predicted
> signal as a heatmap, with the share of the floor that gets a fast or a video-call-ready signal.
> Drag a router and the map follows. Device power can come from a library you build from public
> FCC and ISED certification records. It is pure Rust and egui, MIT OR Apache-2.0, follows the Craft
> standards (clean room, commands for everything, a CLI), and reads files through craft-dxf and
> craft-mesh. A working beta and a SolidCraft prototype exist.

**Website**

*(leave empty, or the repository's README)*

---

## Notes before sending

- **Name check.** "NetCraft" was the first choice, but NETCRAFT is a live US trademark of
  Netcraft Ltd for software and computer services, so it fails the Craft naming standard
  (`standards/naming.md`, "unclear means no"). "RadioCraft", the working name, is unclear too:
  an ARRL-funded Minecraft radio mod and other software projects use it, and Radiocrafts AS makes
  RF radio modules. The maintainer chose to keep RadioCraft (2026-11-22); CoverageCraft and
  AirwaveCraft are the fallbacks. Before sending: run the trademark register search and record it
  in `docs/feature-names.md`, and mention the other uses of the name in the form so the beta team
  can judge.
- **"Wi-Fi"** is a Wi-Fi Alliance trademark. The description uses it only to say what the app
  plans, as the naming standard allows for a technology the app works with. Feature labels say
  "signal", "coverage" and "router" instead.
- **Prior art in the Craft orgs.** A Tangency Labs pull request (tangency-labs/solidcraft#20164,
  draft) adds the same analysis inside SolidCraft. Say in the form or a follow-up whether that
  PR stays, shrinks to the generic pieces (RF materials, slicing) or closes.
- **Open questions to ask the beta team:** whether a beta app should copy
  `standards/app-agents.md` into its `AGENTS.md` now or on acceptance; the app id
  (`ai.storyteller.radiocraft`, as accepted apps use, or your own until then); and whether a
  network-scanning phase (live surveys) is welcome, since it needs OS network access.
