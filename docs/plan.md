# RadioCraft: plan

RadioCraft plans Wi-Fi coverage from floor plans and models made in other Craft apps. It reads a
plan, gives its walls materials, places routers and predicts the signal across the floor. This
page is the plan for the beta and what comes after it.

## Who it is for

- People setting up Wi-Fi at home or in a small office who already have a plan or a model: where
  should the router go, is one enough, which band reaches the bedroom.
- Installers and IT staff who quote small sites and want a coverage picture before they visit.

## Where it fits in the Craft suite

| App | What RadioCraft takes from it |
|---|---|
| CADCraft | DXF floor plans: walls, glazing and doors by layer (`A-WALL`, `A-GLAZ`, `A-DOOR`...), hatches as wall areas, block inserts |
| SolidCraft (Tangency Labs) | 3D models exported as OBJ (one object per body, with its material name) or STL; RadioCraft cuts them at a height |
| craft-libs | `craft-dxf` and `craft-mesh` read those files; nothing is parsed twice |

RadioCraft never edits those apps' files. It keeps its own project file (`.rcraft`, JSON) that
names the plan it was made from, the layer and object materials, the routers and the settings.

## How it predicts the signal

A multi-wall indoor model, the one validated in the SolidCraft prototype:

    RSSI = EIRP − FSPL(1 m, f) − 10 · n · log10(d) − Σ wall losses

- Walls are regions of the plan. Each wall a straight path crosses adds its material's loss
  once; walls of one material that meet count once; overlaps take the larger loss.
- A wall with no material is guessed by its thickness where the path crosses it: under 140 mm
  drywall, else concrete.
- d is the 3D distance; n defaults to 2.2. EIRP is capped at indoor limits by default.
- The floor is the area the walls enclose, with door-sized gaps closed.

Outputs: a heatmap, the share of the floor at −60 dBm or better and at −67 dBm or better, the
5th percentile, and the signal at named points.

## The beta (this repository)

- Open DXF, OBJ and STL plans; a project file.
- Materials by layer or object, with presets (concrete, drywall, glass, low-E glass, metal, wood
  door, none), matched from names by default.
- Routers: add, drag, remove; height and EIRP per band, or a device from a device file (a JSON
  list such as one built from FCC and ISED records).
- Heatmap, colour key, coverage figures; PNG export.
- Every action is a command; `radiocraft-cli` runs the same commands with no window.
- Added from the features common in site-survey tools, in our own design: channels per router;
  maps of the second-strongest signal, the serving router, same-channel routers heard, the
  estimated link rate and leakage outside the walls; a probe listing the strongest routers;
  spot suggestions; measured points with comparison and calibration (exponent and per-band
  offset); an HTML report. Since then: several floors (plans stacked with elevations, offsets and
  slabs, signals crossing them along the straight path).

## After the beta

1. **Calibration of wall losses.** Fit materials' losses too, not only the exponent and offsets,
   when there are enough measured points behind each wall.
2. **Channel suggestions.** Pick channels that keep same-channel routers apart.
3. **Antenna patterns.** A device file can carry a pattern; ceiling and wall mounts differ.
4. **Openings in slabs.** Stairwells and shafts as regions of a floor with no slab.
5. **Site survey** (needs a decision from the Craft maintainers): measure networks around you
   on the computer running RadioCraft, click by click or while walking. It needs OS network
   access, which the other Craft apps don't use.

## Craft conventions it follows

- Pure Rust and egui; MIT OR Apache-2.0.
- Reads files through craft-libs crates pinned at one `rev`.
- Every feature is a command the CLI can run; panels change state only through commands.
- Non-test code doesn't panic; hostile files are bounded by the readers' limits.
- User text goes through `tr()` with English keys, ready for craft-i18n.
- Colours come from one token table (`theme.rs`).
- Clean room: written from published references and its own author's SolidCraft code; no
  other product's code, wording or assets.
- Feature names use our own words: "signal", "coverage", "router", "access point".

## Open items

- The full trademark check of the name (see `beta-application.md`).
- Copying `standards/app-agents.md` into `AGENTS.md` once storytold confirms how beta apps do it.
- An app id and icon.
- Accessibility: keyboard paths for placing and moving routers, and live-region announcements of
  coverage changes, are only partly done in the beta.
