//! Utilitaires en pur Rust.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::checksum::sha256_file;
use crate::jobs::ToolContext;
use crate::output;
use crate::tools::per_file;

/// `utilitaires-empreinte-sha256` : écrit `<fichier>.sha256` à côté de chaque fichier.
pub fn hash_files(ctx: &ToolContext) -> Result<Vec<PathBuf>> {
    per_file(ctx, hash_one)
}

fn hash_one(path: &Path) -> Result<PathBuf> {
    let hash = sha256_file(path)?;
    let name = output::file_name(path);
    let mut out = output::reserve(&output::parent_dir(path), &name, "sha256")?;
    let mut file = out.take_file()?;
    // Format sha256sum : « <empreinte>  <nom> ».
    writeln!(file, "{hash}  {name}")?;
    file.sync_all()?;
    Ok(out.commit())
}

#[cfg(test)]
mod tests {
    use crate::jobs::{JobInput, MemoryReporter, execute};
    use crate::sidecar::Resolver;
    use std::fs;

    #[test]
    fn empreinte_de_bout_en_bout() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("abc.txt");
        fs::write(&a, b"abc").unwrap();
        fs::write(tmp.path().join("abc.txt.sha256"), b"existant").unwrap();
        let outs = execute(
            "utilitaires-empreinte-sha256",
            JobInput::new(vec![a]),
            &MemoryReporter::default(),
            &Resolver::default(),
        )
        .unwrap();
        assert_eq!(outs, vec![tmp.path().join("abc.txt-2.sha256")]);
        assert_eq!(
            fs::read_to_string(&outs[0]).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  abc.txt\n"
        );
        assert_eq!(
            fs::read(tmp.path().join("abc.txt.sha256")).unwrap(),
            b"existant"
        );
    }
}
