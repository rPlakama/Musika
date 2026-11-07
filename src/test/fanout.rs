use super::common::*;
use crate::fanout::*;

#[tokio::test]
async fn maps_every_id_and_collects_errors() {
    let ids: Vec<String> = (0..20).map(|id| id.to_string()).collect();
    let results = map_bounded(ids.clone(), |id| async move {
        if id == "7" {
            anyhow::bail!("failed");
        }
        Ok(id.parse::<usize>().unwrap())
    })
    .await;

    assert_eq!(results.len(), ids.len());
    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for (id, result) in results {
        match result {
            Ok(value) => ok.push((id, value)),
            Err(_) => failed.push(id),
        }
    }
    assert_eq!(failed, vec!["7"]);
    assert_eq!(ok.len(), 19);
    assert!(ok.contains(&("19".to_string(), 19)));
}
