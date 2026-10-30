// Copyright (C) 2026 The Qt Company Ltd.
// SPDX-License-Identifier: LicenseRef-Qt-Commercial OR LGPL-3.0-only

#include "QParserStatusProxyCpp.h"

namespace rust::bridge {

QParserStatusProxyCpp::QParserStatusProxyCpp(QParserStatusProxyRust* rustProxy)
    : QBaseProxy(rustProxy)
{}

QParserStatusProxyCpp::~QParserStatusProxyCpp() = default;

void QParserStatusProxyCpp::classBegin()
{
    m_rustProxy->class_begin();
}

void QParserStatusProxyCpp::componentComplete()
{
    m_rustProxy->component_complete();
}

} // namespace rust::bridge
