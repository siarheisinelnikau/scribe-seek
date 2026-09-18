use crate::transcript::Transcript;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone)]
pub struct Storage {
    pub root: PathBuf,
}

impl Storage {
    pub fn discover() -> Result<Self> {
        let root = if let Some(root) = std::env::var_os("SCRIBE_SEEK_DATA_DIR") {
            PathBuf::from(root)
        } else if std::env::current_exe()?.with_file_name("portable").exists() {
            std::env::current_exe()?.with_file_name("scribe-seek-data")
        } else {
            #[cfg(windows)]
            let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
            #[cfg(not(windows))]
            let base = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share"))
                });
            base.context("Could not locate the data directory")?
                .join("ScribeSeek")
        };
        Self::at(root)
    }

    pub fn at(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("transcripts"))?;
        fs::create_dir_all(root.join("models"))?;
        Ok(Self { root })
    }

    pub fn model_path(&self, id: &str) -> PathBuf {
        self.root.join("models").join(format!("ggml-{id}.bin"))
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn open_folder(path: &Path) -> Result<()> {
        ensure!(path.is_dir(), "Folder is unavailable");
        #[cfg(windows)]
        let mut command = std::process::Command::new("explorer.exe");
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(all(not(windows), not(target_os = "macos")))]
        let mut command = std::process::Command::new("xdg-open");
        command
            .arg(path)
            .spawn()
            .context("Could not open the folder")?;
        Ok(())
    }

    pub fn save(&self, transcript: &Transcript) -> Result<()> {
        transcript.validate()?;
        let id = format!(
            "{:x}",
            Sha256::digest(transcript.source.to_string_lossy().as_bytes())
        );
        atomic_write(
            &self.root.join("transcripts").join(format!("{id}.json")),
            &serde_json::to_vec_pretty(transcript)?,
        )
    }

    pub fn library(&self) -> Result<(Vec<Transcript>, usize)> {
        let mut documents = Vec::new();
        let mut unreadable = 0;
        for entry in fs::read_dir(self.root.join("transcripts"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|s| s != "json") {
                continue;
            }
            let document = (|| -> Result<Transcript> {
                let document: Transcript = serde_json::from_slice(&fs::read(&path)?)?;
                document.validate()?;
                Ok(document)
            })();
            match document {
                Ok(document) => documents.push(document),
                Err(_) => unreadable += 1,
            }
        }
        documents.sort_by_key(|document| std::cmp::Reverse(document.source_modified_ms));
        Ok((documents, unreadable))
    }
}

pub fn fingerprint(path: &Path) -> Result<(u64, u64)> {
    let meta = fs::metadata(path).context("Source file is unavailable")?;
    ensure!(meta.is_file(), "Drop a file, not a folder");
    Ok((
        meta.len(),
        meta.modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    ))
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.partial", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.with_context(|| format!("Could not save {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_replaces_and_ignores_broken_documents() {
        let directory = tempfile::tempdir().unwrap();
        let storage = Storage::at(directory.path().to_owned()).unwrap();
        let mut t = crate::transcript::tests::fixture();
        storage.save(&t).unwrap();
        t.segments[0].text = "Updated".into();
        storage.save(&t).unwrap();
        fs::write(storage.root.join("transcripts/broken.json"), b"broken").unwrap();
        let (docs, broken) = storage.library().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(broken, 1);
        assert_eq!(docs[0].segments[0].text, "Updated");
    }
}
