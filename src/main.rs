mod cert;
mod proxy;

#[tokio::main]
async fn main() {
    let config: proxy::ProxyConfig = Default::default();
    let proxy = config.build();
    let mut events = proxy.subscribe();
    tokio::spawn(async move {
        loop {
            let event = events.recv().await;
            println!("Got event: {:?}", event);
        }
    });
    let _ = proxy::ProxyServer::run(proxy).await.unwrap();
}
