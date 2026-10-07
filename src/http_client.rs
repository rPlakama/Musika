use reqwest;

pub async fn http_call() {
    let body = reqwest::get("https://navidrome.moontier.online") // Currently it will be a bit
        // of hard coding;
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    println!("body = {body:?}");
}
