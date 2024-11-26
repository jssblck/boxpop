//! `boxpop` extracts ("pops") files out of Docker containers ("boxes") and onto the local file system.

#![deny(clippy::unwrap_used)]
#![deny(unsafe_code)]
#![deny(missing_docs)]
#![warn(rust_2018_idioms)]

use color_eyre::{
    eyre::{bail, ensure, Context},
    Report, Result,
};
use derive_more::derive::{Debug, Display};
use oci_client::{manifest::OciImageManifest, Client, Reference};
use std::{path::PathBuf, str::FromStr};

/// Import this with a glob to use all the major types and traits in the library.
pub mod prelude {
    pub use crate::{ImageRef, ImageRefVersion, OutputDir};
}

/// A parsed container image reference.
/// Docs: https://oras.land/docs/concepts/reference/
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
#[debug("{}", self)]
pub struct ImageRef {
    /// The registry in which the image can be found.
    pub registry: String,

    /// The repository of the image.
    pub repository: String,

    /// The version of the image.
    pub version: ImageRefVersion,
}

impl ImageRef {
    /// Resolve the image reference with the backend.
    pub async fn resolve(
        &self,
        client: &Client,
        auth: &Authentication,
    ) -> Result<(OciImageManifest, String)> {
        let (registry, repository, version) = (
            self.registry.clone(),
            self.repository.clone(),
            self.version.clone(),
        );

        let auth = auth.into();
        let reference = match version {
            ImageRefVersion::Tag(tag) => Reference::with_tag(registry, repository, tag),
            ImageRefVersion::Digest(digest) => Reference::with_digest(registry, repository, digest),
        };

        client
            .pull_image_manifest(&reference, &auth)
            .await
            .context("pull image manifest")
    }
}

impl FromStr for ImageRef {
    type Err = Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (registry, s) = s.split_once('/').unwrap_or(("docker.io", s));
        ensure!(!registry.is_empty(), "image registry must be provided");

        if let Some((repository, tag)) = s.rsplit_once(':') {
            ensure!(!repository.is_empty(), "image repository must be provided");
            ensure!(!tag.is_empty(), "image tag must be provided");
            Ok(Self {
                registry: registry.to_string(),
                repository: repository.to_string(),
                version: ImageRefVersion::Tag(tag.to_string()),
            })
        } else if let Some((repository, digest)) = s.rsplit_once('@') {
            ensure!(!repository.is_empty(), "image repository must be provided");
            ensure!(!digest.is_empty(), "image digest must be provided");
            Ok(Self {
                registry: registry.to_string(),
                repository: repository.to_string(),
                version: ImageRefVersion::Digest(digest.to_string()),
            })
        } else {
            Ok(Self {
                registry: registry.to_string(),
                repository: s.to_string(),
                version: ImageRefVersion::Tag(String::from("latest")),
            })
        }
    }
}

impl From<&ImageRef> for oci_client::Reference {
    fn from(image: &ImageRef) -> Self {
        let (registry, repository, version) = (
            image.registry.clone(),
            image.repository.clone(),
            image.version.clone(),
        );
        match version {
            ImageRefVersion::Tag(tag) => Reference::with_tag(registry, repository, tag),
            ImageRefVersion::Digest(digest) => Reference::with_digest(registry, repository, digest),
        }
    }
}

impl std::fmt::Display for ImageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = &self.repository;
        match &self.version {
            ImageRefVersion::Tag(tag) => write!(f, "{name}:{tag}"),
            ImageRefVersion::Digest(digest) => write!(f, "{name}@{digest}"),
        }
    }
}

/// Specifies the verison of an [`ImageRef`].
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display)]
pub enum ImageRefVersion {
    /// The named tag.
    /// Examples: `latest`, `v1.0`, `buster-slim`, etc
    Tag(String),

    /// The indicated digest.
    /// Example: `sha256:abcd1234`
    Digest(String),
}

/// The output directory to which extracted container content is written.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Display, Debug)]
#[debug("{}", self)]
#[display("{}", path.display())]
pub struct OutputDir {
    /// Whether the path was created from a temporary directory.
    pub is_temporary: bool,

    /// The path to the directory.
    pub path: PathBuf,
}

impl OutputDir {
    /// Create a new instance using a temporary directory.
    /// Importantly, this temporary directory is not cleaned up on exit.
    pub fn new_temporary() -> Result<Self> {
        let appid = concat!(env!("CARGO_PKG_NAME"), "_", env!("CARGO_PKG_VERSION"), "_");
        tempfile::TempDir::with_prefix(appid)
            .context("create temp dir")
            .map(|dir| Self {
                is_temporary: true,
                path: dir.into_path(),
            })
    }
}

impl FromStr for OutputDir {
    type Err = Report;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        make_absolute(s)
            .and_then(|path| {
                if path.is_file() {
                    bail!(
                        "output path already exists and is a file: {}",
                        path.display()
                    )
                } else {
                    Ok(path)
                }
            })
            .context("output must not already exist, or must be a directory")
            .map(|path| Self {
                is_temporary: false,
                path,
            })
    }
}

/// Authentication information for the OCI registry.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Display, Default)]
pub enum Authentication {
    /// No authentication information was provided.
    #[default]
    None,

    /// Basic username/password authentication.
    #[display("{}:****", _0)]
    #[debug("Basic({:?}, _)", _0)]
    Basic(String, String),
}

impl Authentication {
    /// Create a new instance with the provided values.
    pub fn new(username: Option<String>, password: Option<String>) -> Self {
        match (username, password) {
            (None, None) => Self::None,
            (Some(username), Some(password)) => Self::Basic(username, password),
            (None, Some(password)) => Self::Basic(String::new(), password),
            (Some(username), None) => Self::Basic(username, String::new()),
        }
    }
}

impl From<&Authentication> for oci_client::secrets::RegistryAuth {
    fn from(auth: &Authentication) -> Self {
        match auth {
            Authentication::None => Self::Anonymous,
            Authentication::Basic(user, pass) => Self::Basic(user.clone(), pass.clone()),
        }
    }
}

fn make_absolute(path: impl Into<PathBuf>) -> Result<PathBuf> {
    let path = path.into();
    std::fs::canonicalize(path).context("canonicalize path using working directory")
}
