// Copyright (C) 2025 The Qt Company Ltd.
// SPDX-License-Identifier: LicenseRef-Qt-Commercial OR LGPL-3.0-only

#include "QObjectProxyCpp.h"

namespace rust::bridge {

QObjectProxyCpp::QObjectProxyCpp(QObjectProxyRust* rustProxy)
    : QBaseProxy(rustProxy)
{}

QObjectProxyCpp::~QObjectProxyCpp() = default;

} // namespace rust::bridge
