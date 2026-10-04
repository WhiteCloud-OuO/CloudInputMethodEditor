//! 版本索引的签名工具。
//!
//!   cloudime-release-sign keygen --out <私钥文件>        # 生成密钥对：私钥写文件（不打印），公钥打印出来填进 cloudime-update
//!   cloudime-release-sign sign <releases.json>           # 私钥取环境变量 CLOUDIME_INDEX_SIGNING_KEY，写 <文件>.sig
//!   cloudime-release-sign verify <releases.json>         # 按程序内置的公钥验 <文件>.sig
//!
//! 私钥与公钥都是 base64 的 32 字节，签名是 base64 的 64 字节。

use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use thiserror::Error;

const KEY_ENV: &str = "CLOUDIME_INDEX_SIGNING_KEY";

#[derive(Parser)]
#[command(name = "cloudime-release-sign")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 生成密钥对
    Keygen {
        /// 私钥写到这个文件（已存在就拒绝）
        #[arg(long)]
        out: PathBuf,
    },

    /// 给文件生成分离签名 <文件>.sig
    Sign { file: PathBuf },

    /// 按内置公钥验 <文件>.sig
    Verify { file: PathBuf },
}

#[derive(Debug, Error)]
enum SignError {
    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("{path} already exists")]
    KeyExists { path: PathBuf },

    #[error("{KEY_ENV} is not set")]
    MissingKey,

    #[error("{KEY_ENV} is not a base64 32-byte key")]
    MalformedKey,

    #[error("signing key does not match any public key built into cloudime-update")]
    ForeignKey,

    #[error("{0}")]
    Verify(#[from] cloudime_update::UpdateError),
}

fn main() {
    if let Err(error) = run(Args::parse().command) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(command: Command) -> Result<(), SignError> {
    match command {
        Command::Keygen { out } => keygen(&out),
        Command::Sign { file } => sign(&file),
        Command::Verify { file } => {
            let signature = std::fs::read_to_string(signature_path(&file))?;
            cloudime_update::verify(&std::fs::read(&file)?, &signature)?;
            println!("{}: signature ok", file.display());
            Ok(())
        }
    }
}

fn keygen(out: &Path) -> Result<(), SignError> {
    if out.exists() {
        return Err(SignError::KeyExists {
            path: out.to_owned(),
        });
    }
    let mut seed = [0u8; 32];
    fill_random(&mut seed)?;
    let key = SigningKey::from_bytes(&seed);
    write_private(out, &STANDARD.encode(seed))?;
    println!("private key: {}", out.display());
    println!(
        "public key:  {}",
        STANDARD.encode(key.verifying_key().to_bytes())
    );
    Ok(())
}

fn sign(file: &Path) -> Result<(), SignError> {
    let encoded = std::env::var(KEY_ENV).map_err(|_| SignError::MissingKey)?;
    let seed: [u8; 32] = STANDARD
        .decode(encoded.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(SignError::MalformedKey)?;
    let key = SigningKey::from_bytes(&seed);
    // 私钥与程序里的公钥对不上时签出来的索引所有客户端都不认，当场拦下
    let public = STANDARD.encode(key.verifying_key().to_bytes());
    if !cloudime_update::PUBLIC_KEYS.contains(&public.as_str()) {
        return Err(SignError::ForeignKey);
    }
    let signature = STANDARD.encode(key.sign(&std::fs::read(file)?).to_bytes());
    let out = signature_path(file);
    std::fs::write(&out, format!("{signature}\n"))?;
    println!("{}", out.display());
    Ok(())
}

fn signature_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

/// 系统随机源：Unix 读 /dev/urandom；Windows 上不生成密钥（密钥离线生成一次）。
fn fill_random(seed: &mut [u8; 32]) -> Result<(), SignError> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(seed)?;
    Ok(())
}

fn write_private(path: &Path, text: &str) -> Result<(), SignError> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(text.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, text)?;
    Ok(())
}
