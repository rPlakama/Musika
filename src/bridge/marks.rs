//! Downloaded/starred mark requests.

use super::*;

impl Bridge {
    pub fn request_marks(&self, songs: Vec<String>, albums: Vec<String>) {
        let generation = self.marks_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx = self.tx.clone();
        crate::app::runtime().spawn(async move {
            let Some(dbs) = crate::app::dbs() else { return };
            match crate::download::window_state(&dbs, &songs, &albums).await {
                Ok(result) => {
                    let _ = tx.send(Event::Marks { generation, result });
                }
                Err(err) => {
                    crate::debug_log!("GPUI: mark window failed: {err:#}");
                }
            }
        });
    }
}
