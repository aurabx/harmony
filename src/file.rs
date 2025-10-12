use crate::utils::Error;
use std::fs;
use std::io::Cursor;
use std::path::Path;

pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), Error> {
    let reader = Cursor::new(bytes);
    let mut zip =
        zip::ZipArchive::new(reader).map_err(|e| Error::from(format!("zip open error: {}", e)))?;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| Error::from(format!("zip idx error: {}", e)))?;
        let outpath = dest.join(file.mangled_name());
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| Error::from(format!("mkdir error: {}", e)))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| Error::from(format!("mkparent error: {}", e)))?;
            }
            let mut outfile = fs::File::create(&outpath)
                .map_err(|e| Error::from(format!("create error: {}", e)))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| Error::from(format!("write error: {}", e)))?;
        }
    }
    Ok(())
}