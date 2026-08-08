#[tokio::main]
async fn main() -> std::io::Result<()> {
    let bind = std::env::var("TEST_SERVICE_BIND").unwrap_or_else(|_| "127.0.0.1:8081".to_string());
    println!("test-service listening on {bind}");
    test_service::run(&bind).await
}
