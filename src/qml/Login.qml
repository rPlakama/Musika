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
            font.pixelSize: root.height * 0.035
            font.bold: true
            font.letterSpacing: root.height * 0.0084
            horizontalAlignment: Text.AlignHCenter
        }

        TextField {
            id: serverURLField
            width: parent.width
            implicitHeight: root.height * 0.049
            placeholderText: qsTr("Server URL")
            placeholderTextColor: Palette.onSurfaceVariant
            color: Palette.onSurface
            font.family: Theme.fonts.body
            font.pixelSize: root.height * 0.0175
            text: Backend.serverURL
            onEditingFinished: Backend.serverURL = text
            background: Rectangle {
                color: Palette.surfaceContainerHigh
                radius: Theme.radius
                border.color: serverURLField.activeFocus ? Palette.primary : Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
        }
        TextField {
            id: usernameField
            width: parent.width
            implicitHeight: root.height * 0.049
            placeholderText: qsTr("Username")
            placeholderTextColor: Palette.onSurfaceVariant
            color: Palette.onSurface
            font.family: Theme.fonts.body
            font.pixelSize: root.height * 0.0175
            text: Backend.username
            onEditingFinished: Backend.username = text
            background: Rectangle {
                color: Palette.surfaceContainerHigh
                radius: Theme.radius
                border.color: usernameField.activeFocus ? Palette.primary : Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
        }
        TextField {
            id: passwordField
            width: parent.width
            implicitHeight: root.height * 0.049
            placeholderText: qsTr("Password")
            placeholderTextColor: Palette.onSurfaceVariant
            color: Palette.onSurface
            font.family: Theme.fonts.body
            font.pixelSize: root.height * 0.0175
            echoMode: TextInput.Password
            text: Backend.password
            onEditingFinished: Backend.password = text
            background: Rectangle {
                color: Palette.surfaceContainerHigh
                radius: Theme.radius
                border.color: passwordField.activeFocus ? Palette.primary : Palette.outlineVariant
                border.width: Math.max(1, root.height * Theme.hairlineRatio)
            }
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
                font.pixelSize: root.height * 0.0175
            }
        }

        FadeText {
            width: parent.width
            wrapMode: Text.WordWrap
            color: Backend.loggedIn ? Palette.primary : Palette.error
            text: Backend.statusMessage
            visible: Backend.statusMessage.length > 0
            horizontalAlignment: Text.AlignHCenter
            font.pixelSize: root.height * 0.014
        }
    }
}
