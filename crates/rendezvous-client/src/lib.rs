//! Fetches, verifies, and caches signed relay bundles from rendezvous. See
//! `docs/RENDEZVOUS_DESIGN.md`. Verification is delegated entirely to
//! `config::verify_bundle` — this crate never trusts a field before that
//! call succeeds.

use async_trait::async_trait;
use common::UnixSeconds;
use config::revocation::RevocationList;
use config::{verify_bundle, ConfigError, RelayBundlePayload, SignedBundle, TrustRoot};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RendezvousClientError {
    #[error("fetch failed: {0}")]
    Fetch(String),
    #[error("verification failed: {0}")]
    Verify(#[from] ConfigError),
    #[error("no cached bundle available")]
    NoCache,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("cache corrupt: {0}")]
    CacheCorrupt(String),
}

/// A source of signed bundles. `HttpSource` is the only implementation
/// today; the trait exists so additional discovery channels (spec §13:
/// "multiple discovery channels") can be added without touching the
/// verify/cache logic.
#[async_trait]
pub trait RendezvousSource: Send + Sync {
    async fn fetch(&self) -> Result<SignedBundle, RendezvousClientError>;
}

pub struct HttpSource {
    pub base_url: String,
    client: reqwest::Client,
}

impl HttpSource {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl RendezvousSource for HttpSource {
    async fn fetch(&self) -> Result<SignedBundle, RendezvousClientError> {
        let url = format!("{}/v1/relay-bundle", self.base_url);
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| RendezvousClientError::Fetch(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(RendezvousClientError::Fetch(format!(
                "http status {}",
                resp.status()
            )));
        }
        resp.json::<SignedBundle>()
            .await
            .map_err(|e| RendezvousClientError::Fetch(e.to_string()))
    }
}

pub struct RendezvousClient<S: RendezvousSource> {
    source: S,
    trust_root: TrustRoot,
    cache_path: PathBuf,
}

impl<S: RendezvousSource> RendezvousClient<S> {
    pub fn new(source: S, trust_root: TrustRoot, cache_path: impl AsRef<Path>) -> Self {
        Self {
            source,
            trust_root,
            cache_path: cache_path.as_ref().to_path_buf(),
        }
    }

    /// Fetch a fresh bundle; on any network failure, fall back to the
    /// last cached (still expiry-checked) bundle — satisfies "rendezvous
    /// temporarily unavailable, cached signed recovery information still
    /// works" (spec §52).
    pub async fn get_bundle(
        &self,
        revoked: &RevocationList,
    ) -> Result<RelayBundlePayload, RendezvousClientError> {
        match self.source.fetch().await {
            Ok(bundle) => {
                let payload =
                    verify_bundle(&bundle, &self.trust_root, revoked, UnixSeconds::now())?;
                self.write_cache(&bundle)?;
                Ok(payload)
            }
            Err(fetch_err) => {
                tracing::warn!(error = %fetch_err, "rendezvous fetch failed, trying cached bundle");
                let cached = self.read_cache()?;
                Ok(verify_bundle(
                    &cached,
                    &self.trust_root,
                    revoked,
                    UnixSeconds::now(),
                )?)
            }
        }
    }

    fn write_cache(&self, bundle: &SignedBundle) -> Result<(), RendezvousClientError> {
        let bytes = serde_json::to_vec(bundle)
            .map_err(|e| RendezvousClientError::CacheCorrupt(e.to_string()))?;
        std::fs::write(&self.cache_path, bytes)?;
        Ok(())
    }

    fn read_cache(&self) -> Result<SignedBundle, RendezvousClientError> {
        if !self.cache_path.exists() {
            return Err(RendezvousClientError::NoCache);
        }
        let bytes = std::fs::read(&self.cache_path)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| RendezvousClientError::CacheCorrupt(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::revocation::RevocationList;
    use config::{EndpointDescriptor, RelayBundlePayload, CURRENT_SCHEMA_VERSION};
    use crypto::hierarchy::KeyCertificate;
    use crypto::KeyPair;

    struct FailingSource;
    #[async_trait]
    impl RendezvousSource for FailingSource {
        async fn fetch(&self) -> Result<SignedBundle, RendezvousClientError> {
            Err(RendezvousClientError::Fetch("simulated outage".into()))
        }
    }

    fn signed_fixture() -> (TrustRoot, SignedBundle, RelayBundlePayload) {
        let root = KeyPair::generate();
        let release = KeyPair::generate();
        let bundle_key = KeyPair::generate();
        let release_cert = KeyCertificate::issue(&root, release.public_key(), 1);
        let bundle_cert = KeyCertificate::issue(&release, bundle_key.public_key(), 2);
        let payload = RelayBundlePayload {
            schema_version: CURRENT_SCHEMA_VERSION,
            issued_at: UnixSeconds::now().0.saturating_sub(10),
            expires_at: UnixSeconds::now().0 + 3600,
            nonce: "n".into(),
            endpoints: vec![EndpointDescriptor {
                id: "relay-1".into(),
                transport: "direct-tls".into(),
                address: "127.0.0.1:9443".into(),
                provider_tag: "dev".into(),
                capabilities: vec!["STREAM".into()],
                cert_sha256_hex: "00".repeat(32),
            }],
        };
        let bundle = SignedBundle::sign(&payload, &bundle_key, bundle_cert, release_cert).unwrap();
        (
            TrustRoot {
                root_public_key: root.public_key(),
            },
            bundle,
            payload,
        )
    }

    #[tokio::test]
    async fn outage_falls_back_to_cached_bundle() {
        let (trust_root, bundle, payload) = signed_fixture();
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("bundle.json");
        std::fs::write(&cache_path, serde_json::to_vec(&bundle).unwrap()).unwrap();

        let client = RendezvousClient::new(FailingSource, trust_root, &cache_path);
        let revoked = RevocationList::empty();
        let got = client.get_bundle(&revoked).await.unwrap();
        assert_eq!(got, payload);
    }

    #[tokio::test]
    async fn outage_with_no_cache_fails_clearly() {
        let (trust_root, _bundle, _payload) = signed_fixture();
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("missing.json");
        let client = RendezvousClient::new(FailingSource, trust_root, &cache_path);
        let revoked = RevocationList::empty();
        let err = client.get_bundle(&revoked).await.unwrap_err();
        assert!(matches!(err, RendezvousClientError::NoCache));
    }
}
