// Copyright (C) 2025 The Qt Company Ltd.
// SPDX-License-Identifier: LicenseRef-Qt-Commercial OR LGPL-3.0-only

pub mod proxy_cpp_bridge;
pub mod proxy_rust;
pub mod proxy_rust_bridge;
pub use proxy_cpp_bridge::ffi::QObjectProxyCpp;
pub use proxy_rust::{QObjectProxyRust, QObjectAdapter};
