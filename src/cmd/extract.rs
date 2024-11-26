//! Extract the contents to a directory on disk.

use std::{path::PathBuf, sync::LazyLock};

use async_tempfile::TempDir;
use boxpop::{prelude::*, Authentication};
use clap::Parser;
use color_eyre::{eyre::Context, Result};
use console::{style, Emoji};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use oci_client::{
    client::{ClientConfig, SizedStream},
    manifest::OciDescriptor,
    secrets::RegistryAuth,
    Client, Reference,
};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    task::JoinSet,
};
use tokio_util::io::StreamReader;

/// Options for the `extract` subcommand.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Parser)]
pub struct Options {
    #[clap(from_global)]
    image: ImageRef,

    /// The username to use when authenticating with the OCI registry.
    #[clap(from_global)]
    username: Option<String>,

    /// The password to use when authenticating with the OCI registry.
    #[clap(from_global)]
    password: Option<String>,

    /// The directory to which the content should be written.
    /// If not set, a temporary directory is created; its path is emitted to stdout.
    #[clap(short, long)]
    output: Option<OutputDir>,
}

impl Options {
    /// Computes final options from the inputs.
    // Consumes so that this method can't unintentionally be called multiple times.
    fn compute(self) -> Result<(ImageRef, OutputDir, Authentication)> {
        self.output
            .map(Ok)
            .unwrap_or_else(OutputDir::new_temporary)
            .context("create temporary output dir")
            .inspect(|output| println!("{output}"))
            .map(|output| {
                let auth = Authentication::new(self.username, self.password);
                (self.image, output, auth)
            })
    }
}

static MAGNIFIER: Emoji<'_, '_> = Emoji("🔍 ", "");
static TRUCK: Emoji<'_, '_> = Emoji("🚚 ", "");
static PACKAGE: Emoji<'_, '_> = Emoji("📦️ ", "");

/// Extracts the contents of the image to disk.
///
/// By default:
/// - Multiplatform images select the "most reasonable" platform based on where this program is running.
/// - All image layers are "squished"; the exported files are the result of applying all layers in order.
//
// Update the docs for the subcommand in `main` if you change this.
pub async fn main(opts: Options) -> Result<()> {
    let client = Client::new(ClientConfig::default());
    let (image, _output, auth) = opts.compute()?;
    let ociref = Reference::from(&image);
    let ociauth = RegistryAuth::from(&auth);

    eprint!(
        "{MAGNIFIER}Resolving manifest for {}...",
        style(image.to_string()).bold().dim()
    );
    let manifest = client
        .pull_image_manifest(&ociref, &ociauth)
        .await
        .map(|(manifest, digest)| {
            eprintln!(" resolved manifest: {}", style(digest).bold().dim(),);
            manifest
        })
        .context("resolve image manifest")?;

    let working = TempDir::new().await.context("create temporary directory")?;
    eprintln!(
        "{PACKAGE}Temporary working directory is {}",
        style(working.display()).bold().dim()
    );

    let layers = manifest.layers;
    let task_count = layers.len() * 2; // Download + apply each layer
    eprintln!(
        "{TRUCK}Pulling {} {}...",
        style(layers.len().to_string()).bold().dim(),
        pluralize("layer", "", "s", layers.len())
    );

    let mut tasks = JoinSet::new();
    let progress = MultiProgress::new();
    for (layer, task) in layers.into_iter().zip(1..) {
        let blob = client
            .pull_blob_stream(&ociref, &layer)
            .await
            .with_context(|| format!("pull layer: {}", layer.digest))?;

        let bar = download_progress(task, task_count, blob.content_length);
        let bar = progress.add(bar);

        tasks.spawn(download_layer(bar, working.dir_path().clone(), layer, blob));
    }

    while let Some(task) = tasks.join_next().await {
        let _downloaded = task.expect("join task").context("download blob")?;
    }

    Ok(())
}

async fn download_layer(
    progress: ProgressBar,
    working: PathBuf,
    layer: OciDescriptor,
    blob: SizedStream,
) -> Result<PathBuf> {
    let name = layer.digest.replace(':', "_");
    let path = working.join(name);
    let mut file = tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("create file: {}", path.display()))?;

    let read = StreamReader::new(blob.stream);
    let read = BufReader::new(read);
    tokio::io::copy(&mut progress.wrap_async_read(read), &mut file)
        .await
        .context("download blob")?;

    file.flush().await.context("flush downloaded blob")?;
    progress.finish_and_clear();

    Ok(path)
}

fn pluralize(base: &str, singular: &str, plural: &str, count: usize) -> String {
    match count {
        1 => format!("{base}{singular}"),
        _ => format!("{base}{plural}"),
    }
}

fn download_progress(task: usize, task_count: usize, bytes: Option<u64>) -> ProgressBar {
    static DOWNLOAD_STYLE: LazyLock<ProgressStyle> = LazyLock::new(|| {
        ProgressStyle::with_template(
            "{prefix:.dim} {eta:.dim} {bar:40.mint/green} {decimal_bytes}/{decimal_total_bytes}",
        )
        .expect("parse progress bar template")
    });

    let bar = match bytes {
        Some(bytes) => ProgressBar::new(bytes).with_style(DOWNLOAD_STYLE.clone()),
        None => ProgressBar::new_spinner(),
    };

    bar.set_prefix(format!("[{task}/{task_count}]"));
    bar
}
