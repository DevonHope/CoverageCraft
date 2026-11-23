//! DWG drawings, read as CADCraft reads them: the `acadrust` crate (MPL-2.0, used unmodified)
//! turns the DWG into a DXF in memory, and RadioCraft reads that DXF as any other.

/// The most a DWG may expand to while it is read (bytes).
const MAX_EXPANDED: usize = 512 << 20;

/// The DXF of a DWG file's bytes. A file it can't read, or a failure inside the reader, is an
/// error, never a crash.
pub fn to_dxf(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 6 || !bytes.starts_with(b"AC") {
        return Err("not a DWG file".into());
    }
    let owned = bytes.to_vec();
    let run = std::panic::catch_unwind(move || -> Result<Vec<u8>, String> {
        let options =
            acadrust::DwgReadOptions::failsafe().with_max_decompressed_bytes(MAX_EXPANDED);
        let mut reader =
            acadrust::DwgReader::from_stream_with_options(std::io::Cursor::new(owned), options);
        let doc = reader
            .read()
            .map_err(|e| format!("the DWG couldn't be read: {e}"))?;
        acadrust::DxfWriter::new(&doc)
            .write_to_vec()
            .map_err(|e| format!("the DWG couldn't be converted: {e}"))
    });
    run.unwrap_or_else(|_| Err("the DWG reader stopped on this file".into()))
}
