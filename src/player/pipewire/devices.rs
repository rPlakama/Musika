// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program; if not, see <https://www.gnu.org/licenses/>.

//! Output-device discovery for the playback settings.
//!
//! The sink itself owns one long-lived PipeWire connection. Device listing is
//! rare (opening the picker, starting with a remembered target), so it opens a
//! short-lived connection, syncs the registry, and quits. The list is sorted
//! by display name so the keyboard picker is deterministic.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pipewire as pw;

/// One PipeWire sink that the transport may target.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutputDevice {
    /// `node.name`, accepted as `target.object` by the sink stream.
    pub id: String,
    /// `node.description` when present, else the node name.
    pub label: String,
}

/// Enumerates `Audio/Sink` nodes. `Err` means PipeWire itself is unavailable;
/// an empty list is a valid session with no sinks.
pub(crate) fn list_output_devices() -> Result<Vec<OutputDevice>, String> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|error| format!("cannot create PipeWire main loop: {error}"))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|error| format!("cannot create PipeWire context: {error}"))?;
    let core = context
        .connect_rc(None)
        .map_err(|error| format!("cannot connect to PipeWire: {error}"))?;
    let registry = core
        .get_registry_rc()
        .map_err(|error| format!("cannot read PipeWire registry: {error}"))?;

    let devices: Rc<RefCell<Vec<OutputDevice>>> = Rc::new(RefCell::new(Vec::new()));
    let collected = Rc::clone(&devices);
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ != pw::types::ObjectType::Node {
                return;
            }
            let Some(props) = global.props else {
                return;
            };
            if props.get(&pw::keys::MEDIA_CLASS) != Some("Audio/Sink") {
                return;
            }
            let Some(id) = props.get(&pw::keys::NODE_NAME).filter(|id| !id.is_empty()) else {
                return;
            };
            let label = props
                .get(&pw::keys::NODE_DESCRIPTION)
                .filter(|label| !label.is_empty())
                .unwrap_or(id);
            collected.borrow_mut().push(OutputDevice {
                id: id.to_string(),
                label: label.to_string(),
            });
        })
        .register();

    let done = Rc::new(Cell::new(false));
    let pending = core.sync(0).map_err(|error| error.to_string())?;
    let quit = mainloop.clone();
    let finished = Rc::clone(&done);
    let _core_listener = core
        .add_listener_local()
        .done(move |id, seq| {
            if id == pw::core::PW_ID_CORE && seq == pending {
                finished.set(true);
                quit.quit();
            }
        })
        .register();

    mainloop.run();

    let mut list = devices.borrow().clone();
    list.sort_by_cached_key(|device| device.label.to_lowercase());
    list.dedup_by(|a, b| a.id == b.id);
    Ok(list)
}
