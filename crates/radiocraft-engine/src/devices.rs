//! Routers and access points a router can name: a few generic ones, and those of a device file
//! (a JSON list of `{id, name?, eirp_dbm: {"2.4", "5", "6": dBm}}`, other keys ignored), such as
//! one built from FCC and ISED certification records. No maker's product ships in the app: their
//! names and figures come from the user's own file.

use crate::materials::Band;

/// A device a router can be.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// EIRP per band before the cap (dBm); `None` where it has no radio.
    pub eirp_dbm: [Option<f64>; 3],
    /// Whether it came from the device file.
    pub from_file: bool,
}

/// The EIRP indoor limits by band (dBm): 30 at 2.4 and 5 GHz, 27 on 6 GHz low-power indoor.
pub const EIRP_CAP_DBM: [f64; 3] = [30.0, 30.0, 27.0];

/// The most a device file may hold, in bytes and in devices.
const MAX_FILE_BYTES: u64 = 4 << 20;
const MAX_FILE_DEVICES: usize = 2000;

/// The generic devices: typical figures for a class of device, not any product's.
pub fn generic() -> Vec<Device> {
    let d = |id: &str, name: &str, eirp: [Option<f64>; 3]| Device {
        id: id.into(),
        name: name.into(),
        eirp_dbm: eirp,
        from_file: false,
    };
    vec![
        d(
            "generic-router",
            "Home router (2.4 and 5 GHz)",
            [Some(20.0), Some(23.0), None],
        ),
        d(
            "generic-router-6",
            "Home router with 6 GHz",
            [Some(20.0), Some(23.0), Some(23.0)],
        ),
        d(
            "generic-ceiling-ap",
            "Ceiling access point",
            [Some(26.0), Some(30.0), Some(27.0)],
        ),
        d(
            "generic-mesh-node",
            "Mesh node",
            [Some(18.0), Some(21.0), Some(21.0)],
        ),
    ]
}

#[derive(serde::Deserialize)]
struct FileDevice {
    id: String,
    #[serde(default)]
    name: Option<String>,
    eirp_dbm: std::collections::BTreeMap<String, f64>,
}

/// The devices of the file at `path`, then the generic ones whose ids it doesn't use.
pub fn load(path: Option<&str>) -> Result<Vec<Device>, String> {
    let mut out: Vec<Device> = Vec::new();
    if let Some(path) = path {
        let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            return Err(format!(
                "{path}: not a device file (a JSON file of at most 4 MB)"
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let list: Vec<FileDevice> = serde_json::from_slice(&bytes)
            .map_err(|e| format!("{path}: a device file is a JSON list of {{id, name?, eirp_dbm: {{\"2.4\", \"5\", \"6\": dBm}}}}: {e}"))?;
        if list.len() > MAX_FILE_DEVICES {
            return Err(format!("{path}: at most {MAX_FILE_DEVICES} devices"));
        }
        for d in list {
            let id = d.id.trim().to_string();
            if id.is_empty()
                || id.chars().count() > 64
                || out.iter().any(|o| o.id.eq_ignore_ascii_case(&id))
            {
                return Err(format!(
                    "{path}: device ids must be 1–64 characters and different, not `{id}`"
                ));
            }
            let mut eirp = [None; 3];
            for (band, v) in &d.eirp_dbm {
                match Band::ALL.into_iter().find(|b| b.name() == band.as_str()) {
                    Some(b) if v.is_finite() && (-30.0..=60.0).contains(v) => {
                        if let Some(slot) = eirp.get_mut(b.index()) {
                            *slot = Some(*v);
                        }
                    }
                    _ => {
                        return Err(format!(
                            "{path}: `{id}` needs eirp_dbm keys \"2.4\", \"5\", \"6\" with -30 to 60 dBm"
                        ));
                    }
                }
            }
            let name = d
                .name
                .filter(|n| !n.trim().is_empty() && n.chars().count() <= 256)
                .unwrap_or_else(|| id.clone());
            out.push(Device {
                id,
                name,
                eirp_dbm: eirp,
                from_file: true,
            });
        }
    }
    for g in generic() {
        if !out.iter().any(|o| o.id.eq_ignore_ascii_case(&g.id)) {
            out.push(g);
        }
    }
    Ok(out)
}
