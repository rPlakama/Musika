import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Theme.js" as Theme

// Library shell: sidebar | main (search + content) | right status bar, with the
// inferior player bar below.
//
// The full "mainscreen" (cover + lyrics) is layered on top of the player.
// "Visual mode" (double-ESC) outlines the four regions.
Item {
    id: root

    property bool nowPlayingExpanded: false
    property bool visualMode: false
    property real playerHeight: height * Theme.playerRatio

    // Actions dispatched from the global key handler.
    function handleAction(action) {
        if (action === "visual_mode") {
            visualMode = !visualMode;
        } else if (action === "back") {
            if (contentStack.depth > 1)
                contentStack.pop();
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.bottomMargin: root.playerHeight
        spacing: 0

        SideBar {
            id: sideBar
            Layout.preferredWidth: root.width * Theme.sidebarRatio
            Layout.fillHeight: true
        }

        ColumnLayout {
            id: mainArea
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            SearchBar {
                Layout.fillWidth: true
            }

            StackView {
                id: contentStack
                Layout.fillWidth: true
                Layout.fillHeight: true
                initialItem: albumListComponent

                Component {
                    id: albumListComponent
                    AlbumList {
                        onAlbumActivated: (albumId, title, artist, year, tracks, coverArt) => {
                            var meta = [];
                            if (year.length > 0)
                                meta.push(year);
                            if (tracks.length > 0)
                                meta.push(tracks + " songs");
                            contentStack.push(albumEntryComponent, {
                                albumId: albumId,
                                albumTitle: title,
                                albumArtist: artist,
                                albumCoverArt: coverArt,
                                albumMeta: meta.join("   ·   ")
                            });
                        }
                    }
                }

                Component {
                    id: albumEntryComponent
                    AlbumEntry {
                        onBack: contentStack.pop()
                    }
                }
            }
        }

        RightBar {
            id: rightBar
            Layout.preferredWidth: root.width * Theme.rightBarRatio
            Layout.fillHeight: true
        }
    }

    InferiorBar {
        id: player
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: root.playerHeight
    }

    NowPlaying {
        anchors.fill: parent
        visible: root.nowPlayingExpanded
    }

    // Visual mode: colourful borders around the four regions.
    Repeater {
        model: [
            { block: sideBar, color: Theme.primary },
            { block: mainArea, color: Theme.tertiary },
            { block: rightBar, color: Theme.secondary },
            { block: player, color: Theme.error }
        ]

        delegate: Rectangle {
            required property var modelData

            x: modelData.block.x
            y: modelData.block.y
            width: modelData.block.width
            height: modelData.block.height
            z: 10
            color: "transparent"
            visible: root.visualMode
            border.width: Math.max(2, root.width * 0.002)
            border.color: modelData.color
        }
    }

    Text {
        anchors.top: parent.top
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.topMargin: root.height * 0.01
        z: 11
        visible: root.visualMode
        text: "VISUAL"
        color: Theme.primary
        font.pixelSize: root.height * 0.02
        font.bold: true
        font.letterSpacing: root.height * 0.006
    }

    // Prime the right status bar with the current counts.
    Component.onCompleted: {
        Backend.refreshLibraryStatus();
        handleAction("visual_mode");
        contentStack.push(albumEntryComponent, { albumId: "al-1", albumTitle: "Test Album" });
    }
}
