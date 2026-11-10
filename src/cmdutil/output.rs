use std::io::Write;

use serde::Serialize;

use super::{Error, Result};

/// Writes v as indented JSON (2 spaces) followed by a newline. HTML is not
/// escaped, so URLs (e.g. invoiceUrl) stay readable.
pub fn print_json<T: Serialize + ?Sized>(w: &mut dyn Write, v: &T) -> Result<()> {
    let mut buf = serde_json::to_vec_pretty(v)
        .map_err(|e| Error::internal(format!("encoding output: {e}")))?;
    buf.push(b'\n');
    w.write_all(&buf)
        .and_then(|()| w.flush())
        .map_err(|e| Error::internal(format!("writing output: {e}")))
}
