//! The renderer's identity: an ed25519 keypair, the same kind iroh uses for
//! node ids, so one identity can be presented over every transport.
//!
//! Keys and signatures travel as lowercase hex. On native hosts the secret
//! key is kept in `<config dir>/hyperb/identity` (64 hex digits).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::fmt;
use std::str::FromStr;

#[derive(Clone)]
pub struct Identity {
    key: SigningKey,
}

impl Identity {
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed).expect("system random number generator");
        Self::from_secret(seed)
    }

    pub fn from_secret(secret: [u8; 32]) -> Self {
        Identity {
            key: SigningKey::from_bytes(&secret),
        }
    }

    pub fn secret(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.key.verifying_key())
    }

    /// Signs `msg`, returning the signature as hex.
    pub fn sign(&self, msg: &[u8]) -> String {
        hex::encode(self.key.sign(msg).to_bytes())
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Identity").field(&self.public()).finish()
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use super::Identity;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    impl Identity {
        /// Loads the identity kept at `path`, creating it if there is none.
        /// A file that exists but cannot be read is an error, never replaced.
        pub fn load_or_create(path: &Path) -> io::Result<Self> {
            match std::fs::read_to_string(path) {
                Ok(s) => {
                    let mut secret = [0u8; 32];
                    hex::decode_to_slice(s.trim(), &mut secret).map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("{}: {e}", path.display()),
                        )
                    })?;
                    Ok(Self::from_secret(secret))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    let id = Self::generate();
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    let mut opts = std::fs::OpenOptions::new();
                    opts.write(true).create_new(true);
                    #[cfg(unix)]
                    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
                    let mut f = opts.open(path)?;
                    writeln!(f, "{}", hex::encode(id.secret()))?;
                    Ok(id)
                }
                Err(e) => Err(e),
            }
        }

        /// The identity in the default location (see [`default_path`]).
        pub fn load_default() -> io::Result<Self> {
            let path = default_path()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
            Self::load_or_create(&path)
        }
    }

    /// `$XDG_CONFIG_HOME/hyperb/identity`, falling back to `~/.config` (or
    /// `%APPDATA%` on Windows).
    pub fn default_path() -> Option<PathBuf> {
        let var = |k| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let base = if cfg!(windows) {
            var("APPDATA")
        } else {
            var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
        };
        Some(base?.join("hyperb").join("identity"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use store::default_path;

/// The public half of an [`Identity`]; displayed and parsed as hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(VerifyingKey);

impl PublicKey {
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// Whether `signature` (hex) is this key's signature of `msg`.
    pub fn verify(&self, msg: &[u8], signature: &str) -> bool {
        let mut sig = [0u8; 64];
        hex::decode_to_slice(signature.trim(), &mut sig).is_ok()
            && self.0.verify(msg, &Signature::from_bytes(&sig)).is_ok()
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0.as_bytes()))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

impl FromStr for PublicKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s.trim(), &mut bytes).map_err(|e| e.to_string())?;
        VerifyingKey::from_bytes(&bytes)
            .map(PublicKey)
            .map_err(|_| "not an ed25519 public key".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let id = Identity::generate();
        let key: PublicKey = id.public().to_string().parse().unwrap();
        assert_eq!(key, id.public());
        let sig = id.sign(b"session");
        assert!(key.verify(b"session", &sig));
        assert!(!key.verify(b"other", &sig));
        assert!(!key.verify(b"session", "zz"));
        assert!(!Identity::generate().public().verify(b"session", &sig));
    }

    #[test]
    fn secret_round_trips() {
        let id = Identity::generate();
        assert_eq!(Identity::from_secret(id.secret()).public(), id.public());
    }

    #[test]
    fn store_creates_then_loads() {
        let dir = std::env::temp_dir().join(format!("hyperb-id-{}", std::process::id()));
        let path = dir.join("sub").join("identity");
        let a = Identity::load_or_create(&path).unwrap();
        let b = Identity::load_or_create(&path).unwrap();
        assert_eq!(a.public(), b.public());
        std::fs::write(&path, "garbage").unwrap();
        assert!(Identity::load_or_create(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
