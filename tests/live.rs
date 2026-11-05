use pin_ci::client_from_env;

// Gate: run with `PIN_CI_LIVE=1 cargo test --test live`
#[tokio::test]
async fn live_docker_hub_alpine() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    use pin_ci::image_ref::ImageRef;
    let c = client_from_env();
    let d1 = c
        .digest(&ImageRef::parse("alpine:3.20").unwrap())
        .await
        .unwrap();
    let d2 = c
        .digest(&ImageRef::parse("alpine:3.20").unwrap())
        .await
        .unwrap();
    assert_eq!(d1, d2);
    assert!(d1.starts_with("sha256:") && d1.len() == 7 + 64, "{d1}");
}

#[tokio::test]
async fn live_docker_hub_ubuntu() {
    if std::env::var("PIN_CI_LIVE").is_err() {
        eprintln!("SKIP live (set PIN_CI_LIVE=1)");
        return;
    }
    use pin_ci::image_ref::ImageRef;
    let d = client_from_env()
        .digest(&ImageRef::parse("ubuntu:24.04").unwrap())
        .await
        .unwrap();
    assert!(d.starts_with("sha256:") && d.len() == 71, "{d}");
}
