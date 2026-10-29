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

import QtQuick
import QtQuick.Controls
import musika
import "Theme.js" as Theme

Item {
    id: root

    Rectangle {
        radius: Theme.radius
        anchors.fill: parent
        color: Palette.background
    }

    Column {
        anchors.centerIn: parent
        width: root.width * 0.224
        spacing: root.height * 0.014

        FadeText {
            width: parent.width
            text: "MUSIKA"
            color: Palette.primary
            font.family: Theme.fonts.wordmark
            font.pixelSize: Theme.fontSize("title", root.height)
            font.bold: true
            font.letterSpacing: root.height * 0.0084
            horizontalAlignment: Text.AlignHCenter
        }

        FormField {
            id: serverURLField
            width: parent.width
            formHeight: root.height
            placeholderText: qsTr("Server URL")
            text: Backend.serverURL
            onEditingFinished: Backend.serverURL = text
        }
        FormField {
            id: usernameField
            width: parent.width
            formHeight: root.height
            placeholderText: qsTr("Username")
            text: Backend.username
            onEditingFinished: Backend.username = text
        }
        FormField {
            id: passwordField
            width: parent.width
            formHeight: root.height
            placeholderText: qsTr("Password")
            echoMode: TextInput.Password
            text: Backend.password
            onEditingFinished: Backend.password = text
        }

        Button {
            id: loginButton
            width: parent.width
            implicitHeight: root.height * 0.049
            enabled: !Backend.busy
            HoverHandler {
                cursorShape: Qt.PointingHandCursor
            }
            text: Backend.busy ? qsTr("Logging in…") : qsTr("Log in")
            onClicked: Backend.login()
            background: Rectangle {
                radius: Theme.radius
                color: !loginButton.enabled
                    ? Palette.surfaceContainerHigh
                    : (loginButton.hovered ? Palette.primary : Palette.primaryContainer)
            }
            contentItem: FadeText {
                text: loginButton.text
                color: Palette.onPrimaryContainer
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                font.pixelSize: Theme.fontSize("body", root.height)
            }
        }

        // AUTH-3: the status is a role container, not bare body copy —
        // errorContainer while logged out, primaryContainer on success. The
        // box reserves its height even when empty so the form never jumps.
        Rectangle {
            id: statusBox
            width: parent.width
            implicitHeight: Math.max(root.height * 0.05,
                statusLabel.implicitHeight + root.height * 0.02)
            color: Backend.statusMessage.length > 0
                ? (Backend.loggedIn ? Palette.primaryContainer : Palette.errorContainer)
                : "transparent"
            border.width: Backend.statusMessage.length > 0
                ? Theme.hairline(root.height)
                : 0
            border.color: Backend.loggedIn ? Palette.primary : Palette.error
            radius: Theme.radius

            FadeText {
                id: statusLabel
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: root.height * 0.01
                anchors.rightMargin: root.height * 0.01
                text: Backend.statusMessage
                color: Backend.loggedIn ? Palette.onPrimaryContainer : Palette.onErrorContainer
                wrapMode: Text.WordWrap
                horizontalAlignment: Text.AlignHCenter
                font.pixelSize: Theme.fontSize("label", root.height)
            }
        }
    }

    // One form-field chrome: the server/user/password boxes differ only in
    // their placeholder, text binding and echo mode. `formHeight` is passed
    // rather than reaching for the page id (inline components keep their own
    // scope).
    component FormField: TextField {
        id: field

        property real formHeight: 0

        implicitHeight: field.formHeight * 0.049
        placeholderTextColor: Palette.onSurfaceVariant
        color: Palette.onSurface
        font.family: Theme.fonts.body
        font.pixelSize: Theme.fontSize("body", field.formHeight)

        background: Rectangle {
            color: Palette.surfaceContainerHigh
            radius: Theme.radius
            border.color: field.activeFocus ? Palette.primary : Palette.outlineVariant
            border.width: Theme.hairline(field.formHeight)
        }
    }
}
