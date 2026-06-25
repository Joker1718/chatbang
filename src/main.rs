//! Binary entry point — a thin wrapper around `chatbang::cli_main()`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    chatbang::cli_main().await
}
