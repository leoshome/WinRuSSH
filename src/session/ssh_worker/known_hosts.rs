use std::path::{Path, PathBuf};

use ssh2::{CheckResult, HashType, HostKeyType, KnownHostFileKind, KnownHostKeyFormat, Session};

use super::login::*;

/// Returns the `known_hosts` file path in the executable's directory (next to `winrussh.ini`).
pub(super) fn default_known_hosts_path() -> PathBuf {
    if let Ok(mut exe_dir) = std::env::current_exe() {
        exe_dir.pop();
        return exe_dir.join("known_hosts");
    }
    PathBuf::from("known_hosts")
}

pub(super) fn known_hosts_entry_name(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{}]:{}", host, port)
    }
}

pub(super) fn base64_encode_nopad(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() * 4 + 2) / 3);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = *chunk.get(1).unwrap_or(&0) as usize;
        let b2 = *chunk.get(2).unwrap_or(&0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) & 0x3f] as char);
        out.push(TABLE[(n >> 12) & 0x3f] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) & 0x3f] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[n & 0x3f] as char);
        }
    }
    out
}

pub(super) fn format_sha256_fingerprint(hash: &[u8]) -> String {
    format!("SHA256:{}", base64_encode_nopad(hash))
}

pub(super) fn host_key_type_label(ty: HostKeyType) -> &'static str {
    match ty {
        HostKeyType::Rsa => "RSA",
        HostKeyType::Dss => "DSA",
        HostKeyType::Ecdsa256 => "ECDSA (nistp256)",
        HostKeyType::Ecdsa384 => "ECDSA (nistp384)",
        HostKeyType::Ecdsa521 => "ECDSA (nistp521)",
        HostKeyType::Ed25519 => "ED25519",
        HostKeyType::Unknown => "UNKNOWN",
    }
}

pub(super) fn to_known_host_key_format(ty: HostKeyType) -> Option<KnownHostKeyFormat> {
    match ty {
        HostKeyType::Rsa => Some(KnownHostKeyFormat::SshRsa),
        HostKeyType::Dss => Some(KnownHostKeyFormat::SshDss),
        HostKeyType::Ecdsa256 => Some(KnownHostKeyFormat::Ecdsa256),
        HostKeyType::Ecdsa384 => Some(KnownHostKeyFormat::Ecdsa384),
        HostKeyType::Ecdsa521 => Some(KnownHostKeyFormat::Ecdsa521),
        HostKeyType::Ed25519 => Some(KnownHostKeyFormat::Ed25519),
        HostKeyType::Unknown => None,
    }
}

pub(super) trait HostKeyBackend {
    fn server_host_key(&self) -> Option<(Vec<u8>, HostKeyType)>;
    fn server_host_key_sha256(&self) -> Option<Vec<u8>>;
    fn check_known_hosts(&self, path: &Path, host: &str, port: u16, key: &[u8]) -> Result<CheckResult, String>;
    fn add_known_host(&self, path: &Path, host: &str, port: u16, key: &[u8], ty: HostKeyType) -> Result<(), String>;
}

impl HostKeyBackend for Session {
    fn server_host_key(&self) -> Option<(Vec<u8>, HostKeyType)> {
        self.host_key().map(|(bytes, ty)| (bytes.to_vec(), ty))
    }

    fn server_host_key_sha256(&self) -> Option<Vec<u8>> {
        self.host_key_hash(HashType::Sha256).map(|b| b.to_vec())
    }

    fn check_known_hosts(&self, path: &Path, host: &str, port: u16, key: &[u8]) -> Result<CheckResult, String> {
        let mut kh = self
            .known_hosts()
            .map_err(|e| format!("Failed to initialize known_hosts: {}", e))?;
        if path.exists() {
            kh.read_file(path, KnownHostFileKind::OpenSSH)
                .map_err(|e| format!("Failed to read known_hosts ({}): {}", path.display(), e))?;
        }
        Ok(kh.check_port(host, port, key))
    }

    fn add_known_host(&self, path: &Path, host: &str, port: u16, key: &[u8], ty: HostKeyType) -> Result<(), String> {
        let format = to_known_host_key_format(ty)
            .ok_or_else(|| "Unsupported host key type for known_hosts".to_string())?;
        let mut kh = self
            .known_hosts()
            .map_err(|e| format!("Failed to initialize known_hosts: {}", e))?;
        if path.exists() {
            kh.read_file(path, KnownHostFileKind::OpenSSH)
                .map_err(|e| format!("Failed to read known_hosts ({}): {}", path.display(), e))?;
        }
        let entry = known_hosts_entry_name(host, port);
        kh.add(&entry, key, "", format)
            .map_err(|e| format!("Failed to add host key to known_hosts: {}", e))?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory for known_hosts: {}", e))?;
            }
        }
        kh.write_file(path, KnownHostFileKind::OpenSSH)
            .map_err(|e| format!("Failed to write known_hosts ({}): {}", path.display(), e))?;
        Ok(())
    }
}

pub(super) fn verify_host_key_with(
    backend: &dyn HostKeyBackend,
    host: &str,
    port: u16,
    io: LoginIo<'_>,
    known_hosts_path: &Path,
    accept_new: bool,
) -> Result<(), String> {
    let sink = io.sink;
    let fail = |msg: String| -> String {
        sink.error(&msg);
        msg
    };

    let (key_bytes, key_type) = backend
        .server_host_key()
        .ok_or_else(|| fail("Server did not provide a host key.".to_string()))?;
    let fingerprint = backend
        .server_host_key_sha256()
        .map(|h| format_sha256_fingerprint(&h))
        .unwrap_or_else(|| "(unavailable)".to_string());

    let check = backend
        .check_known_hosts(known_hosts_path, host, port, &key_bytes)
        .map_err(&fail)?;

    match check {
        CheckResult::Match => Ok(()),
        CheckResult::Mismatch => Err(fail(format!(
            "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED for {}:{}! \
            The {} key fingerprint is {}. Remove the old entry from {} if this change is expected.",
            host,
            port,
            host_key_type_label(key_type),
            fingerprint,
            known_hosts_path.display()
        ))),
        CheckResult::Failure => Err(fail(format!(
            "Host key verification failed while checking {}.",
            known_hosts_path.display()
        ))),
        CheckResult::NotFound => {
            if io.rx.is_none() {
                if accept_new {
                    backend
                        .add_known_host(known_hosts_path, host, port, &key_bytes, key_type)
                        .map_err(&fail)?;
                    sink.warn(&format!(
                        "Permanently added '{}' ({}) to {}.",
                        known_hosts_entry_name(host, port),
                        host_key_type_label(key_type),
                        known_hosts_path.display()
                    ));
                    return Ok(());
                }
                return Err(fail(format!(
                    "Host key for {}:{} ({}) is not in known_hosts. Connect interactively once to trust it, or pass accept_new = true.",
                    host, port, fingerprint
                )));
            }

            sink.write(&format!(
                "\r\nThe authenticity of host '{}' can't be established.\r\n\
                {} key fingerprint is {}.\r\n\
                Are you sure you want to continue connecting (yes/no)? ",
                known_hosts_entry_name(host, port),
                host_key_type_label(key_type),
                fingerprint
            ));

            match read_terminal_line(io, true) {
                Ok(answer) if answer.trim().eq_ignore_ascii_case("yes") => {
                    backend
                        .add_known_host(known_hosts_path, host, port, &key_bytes, key_type)
                        .map_err(&fail)?;
                    sink.warn(&format!(
                        "Warning: Permanently added '{}' ({}) to known_hosts.",
                        known_hosts_entry_name(host, port),
                        host_key_type_label(key_type)
                    ));
                    Ok(())
                }
                Ok(_) => Err(fail("Host key verification failed (user declined).".to_string())),
                Err(LoginAbort::Interrupted) => Err(fail("Host key verification aborted.".to_string())),
                Err(LoginAbort::Disconnected) => Err(fail("Disconnected during host key verification.".to_string())),
            }
        }
    }
}
