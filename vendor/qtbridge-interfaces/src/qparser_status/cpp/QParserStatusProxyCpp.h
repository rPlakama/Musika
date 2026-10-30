// Copyright (C) 2026 The Qt Company Ltd.
// SPDX-License-Identifier: LicenseRef-Qt-Commercial OR LGPL-3.0-only

#ifndef _QPARSERSTATUSPROXYCPP_RUST_BRIDGE__H_
#define _QPARSERSTATUSPROXYCPP_RUST_BRIDGE__H_
#include <QObject>
#include <QQmlParserStatus>
#include <QQmlListProperty>
#include "qtbridge-interfaces/src/cpp/qbaseproxy.h"
#include "qtbridge-interfaces/src/qparser_status/proxy_rust_bridge.rs.h"

namespace rust::bridge {

class QParserStatusProxyCpp : public QObject, public QQmlParserStatus, public QBaseProxy<QParserStatusProxyCpp, QParserStatusProxyRust>
{
public:
    QParserStatusProxyCpp(QParserStatusProxyRust* rustProxy);
    ~QParserStatusProxyCpp();

    // Virtual methods
    void classBegin() override;
    void componentComplete() override;
};

} // namespace rust::bridge

#endif // _QPARSERSTATUSPROXYCPP_RUST_BRIDGE__H_
