//! Minimal EDID parsing (manufacturer + monitor name) used to name outputs.
//!
//! This replaces libdisplay-info so mindwm does not depend on a specific
//! system library version; we only need two strings for wl_output.

use smithay::reexports::drm::control::{connector, Device as ControlDevice};

#[derive(Debug, Clone)]
pub struct EdidInfo {
    pub make: String,
    pub model: String,
}

/// Read and parse the EDID blob attached to a connector, if any.
pub fn for_connector(device: &impl ControlDevice, connector: connector::Handle) -> Option<EdidInfo> {
    let props = device.get_properties(connector).ok()?;
    let (info, value) = props
        .into_iter()
        .filter_map(|(handle, value)| {
            let info = device.get_property(handle).ok()?;
            Some((info, value))
        })
        .find(|(info, _)| info.name().to_str() == Ok("EDID"))?;
    let blob = info.value_type().convert_value(value).as_blob()?;
    let data = device.get_property_blob(blob).ok()?;
    parse(&data)
}

/// Parse the base EDID block (128 bytes). Returns the 3-letter PNP id as the
/// make and the monitor-name descriptor (or the product code) as the model.
pub fn parse(edid: &[u8]) -> Option<EdidInfo> {
    const MAGIC: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
    if edid.len() < 128 || edid[0..8] != MAGIC {
        return None;
    }
    let id = u16::from_be_bytes([edid[8], edid[9]]);
    let letter = |v: u16| -> char {
        let n = (v & 0x1f) as u8;
        if (1..=26).contains(&n) {
            (b'A' + n - 1) as char
        } else {
            '?'
        }
    };
    let make: String = [letter(id >> 10), letter(id >> 5), letter(id)].iter().collect();
    let product = u16::from_le_bytes([edid[10], edid[11]]);

    let mut model = None;
    for off in [54usize, 72, 90, 108] {
        let d = &edid[off..off + 18];
        // Display descriptor with tag 0xFC = monitor name.
        if d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 0xfc {
            let name: String = d[5..18]
                .iter()
                .take_while(|&&b| b != b'\n' && b != 0)
                .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' })
                .collect();
            let name = name.trim().to_string();
            if !name.is_empty() {
                model = Some(name);
                break;
            }
        }
    }
    Some(EdidInfo {
        make,
        model: model.unwrap_or_else(|| format!("{:04X}", product)),
    })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_make_and_name() {
        let mut e = vec![0u8; 128];
        e[0..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        // "DEL" = D(4) E(5) L(12): 00100 00101 01100 -> 0x10AC
        e[8] = 0x10;
        e[9] = 0xac;
        e[54..58].copy_from_slice(&[0, 0, 0, 0xfc]);
        e[59..72].copy_from_slice(b"DELL U2723QE\n");
        let info = parse(&e).unwrap();
        assert_eq!(info.make, "DEL");
        assert_eq!(info.model, "DELL U2723QE");
    }
}
