//! Téléchargement au premier usage des binaires tiers.
//!
//! Le manifeste `scripts/deps.lock.json` (inclus à la compilation) épingle, par
//! plateforme, l'URL, la taille et le SHA-256 de chaque fichier. L'installation
//! se fait dans `<données app>/bin/<id>/` :
//! 1. téléchargement dans un dossier de travail `<données>/bin/.<id>.partiel/` ;
//! 2. vérification de la taille et du SHA-256 **avant** toute extraction
//!    (fichier supprimé et erreur explicite si l'empreinte ne correspond pas) ;
//! 3. extraction sûre (voir [`crate::extract`]) ;
//! 4. activation atomique par renommage du dossier de travail.
//!
//! Aucun autre appel réseau n'existe dans l'application (hors yt-dlp).

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::cancel::CancelToken;
use crate::extract::{self, Hooks};
use crate::sidecar::{self, Binary, RunOptions};

const MANIFEST_JSON: &str = include_str!("../../scripts/deps.lock.json");

/// Type de fichier téléchargé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Kind {
    #[serde(rename = "raw")]
    Raw,
    #[serde(rename = "gz")]
    Gz,
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "tar.xz")]
    TarXz,
    /// Archive ou installeur extrait avec le 7-Zip inclus.
    #[serde(rename = "7z")]
    SevenZip,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileSpec {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub kind: Kind,
    /// Composants de chemin retirés à l'extraction.
    #[serde(default)]
    pub strip: usize,
    /// Destination (relative au dossier d'installation) des fichiers `raw`/`gz`.
    #[serde(default)]
    pub dest: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PlatformSpec {
    #[serde(default)]
    exe: Option<String>,
    #[serde(default)]
    files: Vec<FileSpec>,
    #[serde(default)]
    unavailable: Option<String>,
    /// Liens symboliques de l'archive (jamais extraits) recréés en copies.
    #[serde(default)]
    aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BinarySpec {
    version: String,
    license: String,
    platforms: BTreeMap<String, PlatformSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    schema: u32,
    binaries: BTreeMap<String, BinarySpec>,
    #[serde(default)]
    models: BTreeMap<String, ModelSpec>,
}

fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(MANIFEST_JSON).expect("scripts/deps.lock.json invalide")
    })
}

/// Clé de plateforme du manifeste (`linux-x64`, `windows-x64`, `macos-arm64`…).
pub fn platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "x86_64") => Some("windows-x64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("macos", "aarch64") => Some("macos-arm64"),
        _ => None,
    }
}

/// Ce qui peut être téléchargé pour un binaire sur cette plateforme.
#[derive(Debug, Clone)]
pub struct Offer {
    pub id: &'static str,
    pub version: String,
    pub license: String,
    /// Chemin de l'exécutable relatif au dossier d'installation.
    pub exe: String,
    pub files: Vec<FileSpec>,
    /// Taille totale à télécharger.
    pub size: u64,
    /// Copies à créer après extraction : chemin du lien → chemin de la cible.
    pub aliases: BTreeMap<String, String>,
    /// `false` pour un modèle (fichier de données, jamais rendu exécutable).
    pub executable: bool,
}

/// Offre de téléchargement pour la plateforme courante, ou raison de l'absence.
pub fn offer(binary: Binary) -> Result<Offer, String> {
    let key = platform_key().ok_or("plateforme non prise en charge")?;
    offer_for(binary, key)
}

fn offer_for(binary: Binary, key: &str) -> Result<Offer, String> {
    let spec = manifest().binaries.get(binary.id()).ok_or_else(|| {
        format!(
            "{} n'est pas encore téléchargeable par l'application (prévu en phase 3).",
            binary.label()
        )
    })?;
    let platform = spec
        .platforms
        .get(key)
        .ok_or_else(|| format!("pas de binaire {} pour {key}", binary.label()))?;
    if let Some(reason) = &platform.unavailable {
        return Err(reason.clone());
    }
    let exe = platform
        .exe
        .clone()
        .ok_or_else(|| format!("manifeste incomplet pour {}", binary.id()))?;
    Ok(Offer {
        id: binary.id(),
        version: spec.version.clone(),
        license: spec.license.clone(),
        exe,
        size: platform.files.iter().map(|f| f.size).sum(),
        files: platform.files.clone(),
        aliases: platform.aliases.clone(),
        executable: true,
    })
}

/// Modèle IA du manifeste (`models`) : fichier unique, toutes plateformes.
#[derive(Debug, Clone, Deserialize)]
struct ModelSpec {
    version: String,
    license: String,
    file: String,
    #[serde(default)]
    files: Vec<FileSpec>,
    #[serde(default)]
    unavailable: Option<String>,
}

/// Offre de téléchargement d'un modèle (`id` = clé du manifeste), ou raison.
pub fn model_offer(id: &'static str) -> Result<Offer, String> {
    let spec = manifest()
        .models
        .get(id)
        .ok_or_else(|| format!("modèle inconnu du manifeste : {id}"))?;
    if let Some(reason) = &spec.unavailable {
        return Err(reason.clone());
    }
    Ok(Offer {
        id,
        version: spec.version.clone(),
        license: spec.license.clone(),
        exe: spec.file.clone(),
        size: spec.files.iter().map(|f| f.size).sum(),
        files: spec.files.clone(),
        aliases: BTreeMap::new(),
        executable: false,
    })
}

/// Dossier d'installation `<bin>/<id>`.
pub fn install_dir(bin_dir: &Path, binary: Binary) -> PathBuf {
    bin_dir.join(binary.id())
}

/// Exécutable installé depuis le manifeste, s'il existe.
pub fn installed_exe(bin_dir: &Path, binary: Binary) -> Option<PathBuf> {
    let offer = offer(binary).ok()?;
    let exe = install_dir(bin_dir, binary).join(offer.exe);
    exe.is_file().then_some(exe)
}

/// Source des octets : HTTP en production, mémoire en test.
pub trait Fetch: Sync {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>>;
}

/// Client HTTPS (rustls, aucune configuration système requise).
pub struct Http;

impl Fetch for Http {
    fn open(&self, url: &str) -> Result<Box<dyn Read + Send>> {
        if !url.starts_with("https://") {
            bail!("URL refusée (HTTPS obligatoire) : {url}");
        }
        // Racines de confiance du système (proxy d'entreprise, magasin Windows/macOS).
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_body(None)
            .build()
            .new_agent();
        let response = agent
            .get(url)
            .call()
            .with_context(|| format!("téléchargement impossible : {url}"))?;
        Ok(Box::new(response.into_body().into_reader()))
    }
}

/// Rappels pendant l'installation.
pub struct InstallHooks<'a> {
    /// Octets téléchargés, total attendu.
    pub progress: &'a (dyn Fn(u64, u64) + Sync),
    pub log: &'a (dyn Fn(&str) + Sync),
    pub cancel: Option<&'a CancelToken>,
    /// 7-Zip (bundlé) pour les fichiers de type `7z`.
    pub seven_zip: Option<PathBuf>,
}

/// Télécharge, vérifie et installe `offer` dans `<bin_dir>/<id>/`.
/// Renvoie le chemin de l'exécutable installé.
pub fn install(
    offer: &Offer,
    bin_dir: &Path,
    fetch: &dyn Fetch,
    hooks: &InstallHooks,
) -> Result<PathBuf> {
    fs::create_dir_all(bin_dir).with_context(|| format!("création de {}", bin_dir.display()))?;
    let staging = bin_dir.join(format!(".{}.partiel", offer.id));
    remove_dir_if_exists(&staging)?;
    fs::create_dir_all(&staging)?;
    let result = fill_staging(offer, &staging, fetch, hooks);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    let exe = result?;
    activate(&staging, &bin_dir.join(offer.id))?;
    (hooks.log)(&format!("{} {} installé.", offer.id, offer.version));
    Ok(bin_dir.join(offer.id).join(exe))
}

fn fill_staging(
    offer: &Offer,
    staging: &Path,
    fetch: &dyn Fetch,
    hooks: &InstallHooks,
) -> Result<PathBuf> {
    let downloads = staging.join(".telechargements");
    fs::create_dir_all(&downloads)?;
    let mut done = 0;
    for (i, file) in offer.files.iter().enumerate() {
        let local = downloads.join(format!("{i}"));
        (hooks.log)(&format!("Téléchargement de {}…", file.url));
        download_verified(file, &local, fetch, hooks, (done, offer.size))?;
        done += file.size;
        unpack(file, &local, staging, hooks)?;
        fs::remove_file(&local)?;
    }
    fs::remove_dir_all(&downloads)?;
    create_aliases(staging, &offer.aliases)?;
    let exe = PathBuf::from(&offer.exe);
    let full = staging.join(&exe);
    if !full.is_file() {
        bail!("exécutable {} absent de l'archive téléchargée", offer.exe);
    }
    if offer.executable {
        make_executable(&full)?;
    }
    Ok(exe)
}

/// Recrée en copies les liens symboliques ignorés à l'extraction
/// (ex. `libqpdf.so.30` → `libqpdf.so.30.4.2`). Chemins relatifs sûrs uniquement.
fn create_aliases(staging: &Path, aliases: &BTreeMap<String, String>) -> Result<()> {
    for (link, target) in aliases {
        let safe = |p: &str| {
            extract::safe_relative(Path::new(p), 0)
                .with_context(|| format!("chemin invalide dans le manifeste : {p}"))
        };
        let (link, target) = (staging.join(safe(link)?), staging.join(safe(target)?));
        if !link.exists() {
            fs::copy(&target, &link)
                .with_context(|| format!("copie de {} impossible", target.display()))?;
        }
    }
    Ok(())
}

/// Remplace l'installation précédente (mise à jour) par le dossier de travail.
fn activate(staging: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        let old = target.with_extension("ancien");
        remove_dir_if_exists(&old)?;
        fs::rename(target, &old).context("remplacement de l'ancienne version impossible")?;
        let _ = fs::remove_dir_all(&old);
    }
    fs::rename(staging, target).context("activation du binaire impossible")
}

fn remove_dir_if_exists(dir: &Path) -> Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("suppression de {}", dir.display()))
        }
        _ => Ok(()),
    }
}

/// Erreur d'empreinte : message destiné à l'utilisateur.
fn hash_error(file: &FileSpec, actual: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "Empreinte SHA-256 invalide pour {} : attendue {}, obtenue {}. \
         Le fichier a été supprimé (téléchargement corrompu ou modifié).",
        file.url,
        file.sha256,
        actual
    )
}

/// Télécharge `file` dans `dest` en vérifiant taille et SHA-256 au fil de l'eau.
fn download_verified(
    file: &FileSpec,
    dest: &Path,
    fetch: &dyn Fetch,
    hooks: &InstallHooks,
    (offset, total): (u64, u64),
) -> Result<()> {
    let mut reader = fetch.open(&file.url)?;
    let mut out = BufWriter::new(OpenOptions::new().write(true).create_new(true).open(dest)?);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    loop {
        if let Some(c) = hooks.cancel {
            c.check()?;
        }
        let n = reader.read(&mut buf).context("connexion interrompue")?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > file.size {
            bail!(
                "{} : taille supérieure à la taille épinglée ({} octets)",
                file.url,
                file.size
            );
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        (hooks.progress)(offset + received, total);
    }
    out.flush()?;
    drop(out);
    let actual = crate::checksum::to_hex(&hasher.finalize());
    if !crate::checksum::matches(&actual, &file.sha256) {
        let _ = fs::remove_file(dest);
        return Err(hash_error(file, &actual));
    }
    if received != file.size {
        bail!("{} : taille {received} au lieu de {}", file.url, file.size);
    }
    Ok(())
}

fn dest_name(file: &FileSpec) -> String {
    file.dest.clone().unwrap_or_else(|| {
        let name = file.url.rsplit('/').next().unwrap_or("fichier");
        name.trim_end_matches(".gz").to_string()
    })
}

/// Chemin de destination sûr (relatif, sans `..`) pour `raw`/`gz`.
fn dest_path(staging: &Path, file: &FileSpec) -> Result<PathBuf> {
    let name = dest_name(file);
    let rel = extract::safe_relative(Path::new(&name), 0)
        .with_context(|| format!("destination invalide dans le manifeste : {name}"))?;
    let path = staging.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(path)
}

fn unpack(file: &FileSpec, local: &Path, staging: &Path, hooks: &InstallHooks) -> Result<()> {
    let warn = |m: &str| (hooks.log)(m);
    let ex = Hooks {
        warn: &warn,
        progress: &|_| {},
        cancel: hooks.cancel,
    };
    match file.kind {
        Kind::Raw => {
            fs::copy(local, dest_path(staging, file)?)?;
        }
        Kind::Gz => {
            let mut gz = flate2::read::GzDecoder::new(BufReader::new(File::open(local)?));
            let mut out = File::create(dest_path(staging, file)?)?;
            io::copy(&mut gz, &mut out).context("décompression gzip impossible")?;
        }
        Kind::Zip => {
            extract::zip(local, staging, file.strip, &ex)?;
        }
        Kind::TarGz => {
            let gz = flate2::read::GzDecoder::new(BufReader::new(File::open(local)?));
            untar(gz, staging, file.strip, &ex)?;
        }
        Kind::TarXz => {
            let tar_path = local.with_extension("tar");
            xz_to_file(local, &tar_path)?;
            untar(
                BufReader::new(File::open(&tar_path)?),
                staging,
                file.strip,
                &ex,
            )?;
            fs::remove_file(&tar_path)?;
        }
        Kind::SevenZip => seven_zip_extract(local, staging, hooks)?,
    }
    Ok(())
}

fn untar<R: Read>(reader: R, dest: &Path, strip: usize, hooks: &Hooks) -> Result<()> {
    let position = std::cell::Cell::new(0);
    extract::tar(reader, dest, strip, hooks, (&position, 1))?;
    Ok(())
}

fn xz_to_file(src: &Path, dest: &Path) -> Result<()> {
    let mut input = BufReader::new(File::open(src)?);
    let mut out = BufWriter::new(File::create(dest)?);
    lzma_rs::xz_decompress(&mut input, &mut out)
        .map_err(|e| anyhow::anyhow!("décompression xz impossible : {e}"))?;
    out.flush()?;
    Ok(())
}

fn seven_zip_extract(archive: &Path, dest: &Path, hooks: &InstallHooks) -> Result<()> {
    let seven = hooks
        .seven_zip
        .as_deref()
        .context("7-Zip (inclus dans l'application) est nécessaire pour cette extraction")?;
    let mut out_arg = std::ffi::OsString::from("-o");
    out_arg.push(dest);
    let args = [
        "x".into(),
        "-y".into(),
        "-aos".into(),
        out_arg,
        archive.as_os_str().to_owned(),
    ];
    let opts = RunOptions {
        cancel: hooks.cancel,
        ..RunOptions::timeout(Duration::from_secs(600))
    };
    sidecar::run_with(seven, &args, opts)?.check("7-Zip")?;
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Vérifie la cohérence du manifeste (utilisé par les tests et la CI).
pub fn manifest_problems() -> Vec<String> {
    let m = manifest();
    let mut problems = Vec::new();
    if m.schema != 1 {
        problems.push(format!("schéma inconnu : {}", m.schema));
    }
    for (id, spec) in &m.binaries {
        for (key, p) in &spec.platforms {
            problems.extend(platform_problems(id, key, p));
        }
    }
    for (id, spec) in &m.models {
        let as_platform = PlatformSpec {
            exe: Some(spec.file.clone()),
            files: spec.files.clone(),
            unavailable: spec.unavailable.clone(),
            aliases: BTreeMap::new(),
        };
        problems.extend(platform_problems(id, "modele", &as_platform));
    }
    problems
}

fn platform_problems(id: &str, key: &str, p: &PlatformSpec) -> Vec<String> {
    let mut problems = Vec::new();
    if p.unavailable.is_some() {
        return problems;
    }
    if p.exe.is_none() || p.files.is_empty() {
        problems.push(format!("{id}/{key} : exe ou fichiers manquants"));
    }
    for f in &p.files {
        let hex = f.sha256.len() == 64 && f.sha256.bytes().all(|b| b.is_ascii_hexdigit());
        if !hex || !f.url.starts_with("https://") || f.size == 0 {
            problems.push(format!("{id}/{key} : entrée invalide {}", f.url));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Mutex;

    struct Memory(HashMap<String, Vec<u8>>);

    impl Fetch for Memory {
        fn open(&self, url: &str) -> Result<Box<dyn Read + Send>> {
            let data = self.0.get(url).context("URL inconnue")?.clone();
            Ok(Box::new(Cursor::new(data)))
        }
    }

    fn spec(url: &str, data: &[u8], kind: Kind) -> FileSpec {
        FileSpec {
            url: url.to_string(),
            sha256: crate::checksum::sha256_reader(data).unwrap(),
            size: data.len() as u64,
            kind,
            strip: 0,
            dest: None,
        }
    }

    fn offer(exe: &str, files: Vec<FileSpec>) -> Offer {
        Offer {
            id: "outil",
            version: "1.0".into(),
            license: "MIT".into(),
            exe: exe.into(),
            size: files.iter().map(|f| f.size).sum(),
            files,
            aliases: BTreeMap::new(),
            executable: true,
        }
    }

    fn run_install(o: &Offer, bin: &Path, fetch: &Memory) -> Result<PathBuf> {
        let progress = Mutex::new(Vec::new());
        let hooks = InstallHooks {
            progress: &|d, t| progress.lock().unwrap().push((d, t)),
            log: &|_| {},
            cancel: None,
            seven_zip: None,
        };
        let r = install(o, bin, fetch, &hooks);
        if r.is_ok() {
            let p = progress.lock().unwrap();
            assert_eq!(p.last().map(|x| x.0), Some(o.size), "progression finale");
        }
        r
    }

    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, data) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn manifeste_coherent_et_complet() {
        assert!(manifest_problems().is_empty(), "{:?}", manifest_problems());
        for key in ["linux-x64", "windows-x64", "macos-x64", "macos-arm64"] {
            // 7-Zip doit être obtenable partout : il est inclus dans l'installeur.
            assert!(offer_for(Binary::SevenZip, key).is_ok(), "{key}");
        }
        let gs = offer_for(Binary::Ghostscript, "linux-x64").unwrap_err();
        assert!(gs.contains("ghostscript"), "{gs}");
        for key in ["linux-x64", "windows-x64", "macos-x64", "macos-arm64"] {
            assert!(offer_for(Binary::Qpdf, key).is_ok(), "qpdf {key}");
        }
    }

    #[test]
    fn installation_tar_gz_avec_strip_et_brut() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[
            ("outil-1.0/bin/outil", b"#!binaire"),
            ("outil-1.0/LISEZMOI", b"doc"),
        ]);
        let mut a = spec("https://exemple.test/outil.tar.gz", &archive, Kind::TarGz);
        a.strip = 1;
        let mut extra = spec("https://exemple.test/donnees.bin", b"donnees", Kind::Raw);
        extra.dest = Some("share/donnees.bin".into());
        let fetch = Memory(HashMap::from([
            (a.url.clone(), archive.clone()),
            (extra.url.clone(), b"donnees".to_vec()),
        ]));
        let o = offer("bin/outil", vec![a, extra]);
        let exe = run_install(&o, tmp.path(), &fetch).unwrap();
        assert_eq!(exe, tmp.path().join("outil/bin/outil"));
        assert_eq!(fs::read(&exe).unwrap(), b"#!binaire");
        assert_eq!(
            fs::read(tmp.path().join("outil/share/donnees.bin")).unwrap(),
            b"donnees"
        );
        assert!(!tmp.path().join(".outil.partiel").exists());
        assert!(sidecar::is_executable(&exe));
    }

    #[test]
    fn empreinte_invalide_rien_n_est_installe() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = spec("https://exemple.test/outil", b"vrai contenu", Kind::Raw);
        f.dest = Some("outil".into());
        let fetch = Memory(HashMap::from([(f.url.clone(), b"faux contenu".to_vec())]));
        let err = run_install(&offer("outil", vec![f]), tmp.path(), &fetch).unwrap_err();
        assert!(
            err.to_string().contains("Empreinte SHA-256 invalide"),
            "{err}"
        );
        assert!(!tmp.path().join("outil").exists());
        assert!(!tmp.path().join(".outil.partiel").exists());
    }

    #[test]
    fn taille_superieure_refusee() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = spec("https://exemple.test/outil", b"abc", Kind::Raw);
        f.dest = Some("outil".into());
        let fetch = Memory(HashMap::from([(f.url.clone(), b"abcdef".to_vec())]));
        let err = run_install(&offer("outil", vec![f]), tmp.path(), &fetch).unwrap_err();
        assert!(err.to_string().contains("taille"), "{err}");
    }

    #[test]
    fn gz_zip_xz_et_mise_a_jour() {
        let tmp = tempfile::tempdir().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"v1").unwrap();
        let gz = gz.finish().unwrap();
        let mut f = spec("https://exemple.test/outil.gz", &gz, Kind::Gz);
        f.dest = Some("outil".into());
        let fetch = Memory(HashMap::from([(f.url.clone(), gz)]));
        run_install(&offer("outil", vec![f]), tmp.path(), &fetch).unwrap();

        // Mise à jour (ex. yt-dlp) : la nouvelle version remplace l'ancienne.
        let mut zip_data = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut zip_data));
            w.start_file("racine/outil", zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(b"v2").unwrap();
            w.finish().unwrap();
        }
        let mut z = spec("https://exemple.test/outil.zip", &zip_data, Kind::Zip);
        z.strip = 1;
        let fetch = Memory(HashMap::from([(z.url.clone(), zip_data)]));
        let exe = run_install(&offer("outil", vec![z]), tmp.path(), &fetch).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"v2");

        let mut tar_data = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_data);
            let mut h = tar::Header::new_gnu();
            h.set_size(2);
            h.set_cksum();
            b.append_data(&mut h, "outil", &b"v3"[..]).unwrap();
            b.finish().unwrap();
        }
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut &tar_data[..], &mut xz).unwrap();
        let x = spec("https://exemple.test/outil.tar.xz", &xz, Kind::TarXz);
        let fetch = Memory(HashMap::from([(x.url.clone(), xz)]));
        let exe = run_install(&offer("outil", vec![x]), tmp.path(), &fetch).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"v3");
    }

    #[test]
    fn exe_absent_et_7z_sans_7zip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = spec("https://exemple.test/autre", b"x", Kind::Raw);
        f.dest = Some("autre".into());
        let fetch = Memory(HashMap::from([(f.url.clone(), b"x".to_vec())]));
        let err = run_install(&offer("outil", vec![f]), tmp.path(), &fetch).unwrap_err();
        assert!(err.to_string().contains("absent"), "{err}");

        let s = spec("https://exemple.test/setup.exe", b"MZ", Kind::SevenZip);
        let fetch = Memory(HashMap::from([(s.url.clone(), b"MZ".to_vec())]));
        let err = run_install(&offer("outil", vec![s]), tmp.path(), &fetch).unwrap_err();
        assert!(err.to_string().contains("7-Zip"), "{err}");
    }

    #[test]
    fn alias_recree_en_copie() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[("bin/outil", b"#!x"), ("lib/libx.so.1.2", b"ELF")]);
        let a = spec("https://exemple.test/o.tar.gz", &archive, Kind::TarGz);
        let fetch = Memory(HashMap::from([(a.url.clone(), archive.clone())]));
        let mut o = offer("bin/outil", vec![a]);
        o.aliases
            .insert("lib/libx.so.1".into(), "lib/libx.so.1.2".into());
        run_install(&o, tmp.path(), &fetch).unwrap();
        assert_eq!(
            fs::read(tmp.path().join("outil/lib/libx.so.1")).unwrap(),
            b"ELF"
        );
        o.aliases
            .insert("../evasion".into(), "lib/libx.so.1.2".into());
        assert!(run_install(&o, tmp.path(), &fetch).is_err());
    }

    #[test]
    fn destination_dangereuse_refusee() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = spec("https://exemple.test/x", b"x", Kind::Raw);
        f.dest = Some("../evasion".into());
        let fetch = Memory(HashMap::from([(f.url.clone(), b"x".to_vec())]));
        assert!(run_install(&offer("outil", vec![f]), tmp.path(), &fetch).is_err());
        assert!(!tmp.path().parent().unwrap().join("evasion").exists());
    }

    #[test]
    fn annulation_pendant_le_telechargement() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = spec("https://exemple.test/outil", b"abc", Kind::Raw);
        f.dest = Some("outil".into());
        let fetch = Memory(HashMap::from([(f.url.clone(), b"abc".to_vec())]));
        let token = CancelToken::default();
        token.cancel();
        let hooks = InstallHooks {
            progress: &|_, _| {},
            log: &|_| {},
            cancel: Some(&token),
            seven_zip: None,
        };
        let err = install(&offer("outil", vec![f]), tmp.path(), &fetch, &hooks).unwrap_err();
        assert!(crate::cancel::is_cancelled(&err));
        assert!(!tmp.path().join("outil").exists());
    }

    #[test]
    fn http_refuse_les_url_non_https() {
        assert!(Http.open("http://exemple.test/x").is_err());
    }
}
