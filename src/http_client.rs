use reqwest;

pub async fn http_call() {
    let body = reqwest::get("https://docs.rs/reqwest/latest/reqwest")
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    println!("body = {body:?}");
}
