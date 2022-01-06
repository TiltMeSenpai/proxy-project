mod cert;
mod proxy;

#[tokio::main]
async fn main() {
    let config: proxy::ProxyConfig = Default::default();
    let proxy = config.build();
    let _ = proxy::ProxyServer::run(proxy).await.unwrap();
}
