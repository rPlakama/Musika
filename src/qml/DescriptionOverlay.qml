// Musika — a keyboard-driven music player.
// Copyright (C) 2026 rPlakama <rPlakama@proton.me>
//
// This program is free software; you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation; either version 2 of the License, or
// (at your option) any later version.

import QtQuick
import QtQuick.Layouts
import musika
import "Theme.js" as Theme

// Full-window reader for album notes / artist biography. It deliberately
// mirrors the lyrics overlay's reading grammar without carrying synced-lyric
// timing state.
Item {
    id: root

    property bool shown: false
    property string title: ""
    property string subtitle: ""
    property string body: ""

    signal dismissed()

    anchors.fill: parent
    visible: shown || scrim.opacity > 0
    z: Theme.zOverlay

    function open(nextTitle, nextSubtitle, nextBody) {
        if (!nextBody || nextBody.length === 0)
            return;
        root.title = nextTitle;
        root.subtitle = nextSubtitle;
        root.body = nextBody;
        root.shown = true;
    }

    function close() {
        root.shown = false;
    }

    function toggle(title, subtitle, body) {
        if (root.shown)
            root.close();
        else
            root.open(title, subtitle, body);
    }

    function scrollBy(delta) {
        var max = Math.max(0, view.contentHeight - view.height);
        view.contentY = Math.max(0, Math.min(max, view.contentY + delta));
    }

    function handleAction(action) {
        if (action === "back" || action === "cancel") {
            root.close();
            return true;
        }
        if (action === "move_down")
            root.scrollBy(Math.max(18, text.font.pixelSize * text.lineHeight));
        else if (action === "move_up")
            root.scrollBy(-Math.max(18, text.font.pixelSize * text.lineHeight));
        else if (action === "page_down" || action === "quick_page_down")
            root.scrollBy(view.height * 0.9);
        else if (action === "page_up" || action === "quick_page_up")
            root.scrollBy(-view.height * 0.9);
        else if (action === "top")
            view.contentY = 0;
        else if (action === "bottom")
            view.contentY = Math.max(0, view.contentHeight - view.height);
        else
            return false;
        return true;
    }

    Scrim {
        id: scrim
        shown: root.shown
        tapToDismiss: true
        onDismissed: root.close()
    }

    Rectangle {
        id: panel
        anchors.centerIn: parent
        width: Math.min(parent.width * 0.72, 720)
        height: parent.height
        color: Qt.rgba(0, 0, 0, 0.72)
        radius: Theme.radius

        ColumnLayout {
            anchors.fill: parent
            anchors.margins: root.width * 0.02
            spacing: root.height * 0.012

            FadeText {
                Layout.fillWidth: true
                text: root.title
                color: Palette.onSurface
                font.pixelSize: root.height * 0.035
                font.bold: true
                elide: Text.ElideRight
            }
            FadeText {
                Layout.fillWidth: true
                visible: root.subtitle.length > 0
                text: root.subtitle
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.02
                elide: Text.ElideRight
            }

            Flickable {
                id: view
                Layout.fillWidth: true
                Layout.fillHeight: true
                contentWidth: width
                contentHeight: text.implicitHeight
                boundsBehavior: Flickable.StopAtBounds
                clip: true

                FadeText {
                    id: text
                    width: view.width
                    text: root.body
                    color: Palette.onSurface
                    font.pixelSize: root.height * 0.022
                    lineHeight: 1.35
                    wrapMode: Text.WordWrap
                    maximumLineCount: 0
                }
            }

            FadeText {
                Layout.fillWidth: true
                text: "j/k scroll   ·   Esc/Q close"
                color: Palette.onSurfaceVariant
                font.pixelSize: root.height * 0.018
                horizontalAlignment: Text.AlignRight
            }
        }
    }
}
