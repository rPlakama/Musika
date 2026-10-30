// Copyright (C) 2025 The Qt Company Ltd.
// SPDX-License-Identifier: LicenseRef-Qt-Commercial OR LGPL-3.0-only

#ifndef _QOBJECTPROXYCPP_RUST_BRIDGE__H_
#define _QOBJECTPROXYCPP_RUST_BRIDGE__H_
#include <QObject>
#include <QQmlListProperty>
#include "qtbridge-interfaces/src/cpp/qbaseproxy.h"
#include "qtbridge-interfaces/src/qobject/proxy_rust_bridge.rs.h"

namespace rust::bridge {

class QObjectProxyCpp : public QObject, public QBaseProxy<QObjectProxyCpp, QObjectProxyRust>
{
public:
    QObjectProxyCpp(QObjectProxyRust* rustProxy);
    ~QObjectProxyCpp();
};

} // namespace rust::bridge

#endif // _QOBJECTPROXYCPP_RUST_BRIDGE__H_
