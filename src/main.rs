#[cfg(any(feature = "http", feature = "grpc"))]
#[tokio::main]
async fn main() {
    if let Err(e) = stratum::runtime::run_async().await {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(all(not(feature = "http"), not(feature = "grpc")))]
fn main() {
    if let Err(e) = stratum::runtime::run_sync() {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}
