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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

// ARC-UI-1: the option boxes are an inline component whose geometry binds to
// the enclosing `board` id; Bound makes those enclosing ids resolvable.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Window
import musika
import "Theme.js" as Theme

// ARC-43/ARC-UI-1: Alt-hold quick actions, keyboard gesture. Holding Alt
// overlays four fixed verbs around the current item; `h`/`j`/`k`/`l` (or the
// arrow keys) aim west/south/north/east and releasing Alt is Enter (it fires
// the aimed action). The center is the neutral aim: releasing there fires
// nothing — that IS the cancel, Esc is not involved (it is swallowed while
// Alt is held) — and aiming toward the opposite quadrant first passes through
// neutral (north + `j` = neutral, `j` again = south). Sticky menus (a mouse
// click arms them) use the same board: clicks on a tile fire it and a click
// outside the board cancels. The host arms it with the current item and a
// `{north,south,east,west}` config of `{label, icon, invoke}` slots (null =
// open, drawn as "—"; the verb is its SVG glyph); one `board` rectangle owns
// the margin and scale and the tiles anchor to its edges. The option tiles
// are acrylic panels (blurred backdrop, like the search overlay).
Item {
    id: root

    /// UI-60: a rating step happened at (x, y) in this overlay's
    /// coordinates — the host spawns the bright star burst there.
    signal ratingStepped(real x, real y)

    property bool armed: false
    property Item targetItem: null
    property var config: null
    property string aimed: ""
    /// UI-60: the live rating level while the favourite slot is adjusted
    /// with Shift-J/K (seeded from the slot's own `rating` on arm).
    property int ratingValue: 0
    /// A rating step was taken in this hold: the release must not also
    /// toggle the favourite underneath it.
    property bool ratingTouched: false
    /// The level is not on the server yet (the idle timer or the release
    /// flushes it once).
    property bool ratingDirty: false

    // The steps commit on a short idle: a burst of Shift-K/J presses sends
    // one request (the server cannot reorder parallel ratings), while the
    // badge previews every step immediately.
    Timer {
        id: ratingCommit
        interval: 180
        repeat: false
        onTriggered: root.commitRating()
    }
    /// UI-55: the item behind the option tiles whose pixels feed the acrylic
    /// blur (the shell, or the fullscreen player when it is up).
    property Item backdrop: null
    /// Esc retired this hold; arming stays off until Alt is released.
    property bool dismissed: false
    /// Sticky menus (the wipe chooser) skip the Alt-hold gesture: they stay
    /// armed until Enter fires the aim or Esc cancels.
    property bool sticky: false

    /// ARC-I: the search column arms this too, so an open search no longer
    /// disarms the gesture (only a half-typed sequence does).
    readonly property bool available: !Backend.keyPending

    readonly property point anchor: (targetItem && targetItem.visible && targetItem.width > 0)
        ? targetItem.mapToItem(root, targetItem.width / 2, targetItem.height / 2)
        : Qt.point(-1000, -1000)
    readonly property real targetWidth: targetItem ? targetItem.width : 0
    readonly property real targetHeight: targetItem ? targetItem.height : 0

    /// ARC-UI-1: the window's content rectangle in this item's coordinates;
    /// the board clamps to it so no tile is cut by the window edge. Falls back
    /// to the item's own bounds before the window exists.
    readonly property rect windowRect: {
        var win = Window.window;
        if (!win || !win.contentItem)
            return Qt.rect(0, 0, root.width, root.height);
        var tl = win.contentItem.mapToItem(root, 0, 0);
        var br = win.contentItem.mapToItem(root, win.contentItem.width, win.contentItem.height);
        return Qt.rect(tl.x, tl.y, br.x - tl.x, br.y - tl.y);
    }

    /// Spawn motion: the shared spring pop (ARC-UI-2's `PopFx`, lifted from
    /// this board) — the cluster zooms out of the target on one overshoot and
    /// settle, and collapses back with a plain ease-in; visibility follows
    /// the animated value so the collapse plays out.
    readonly property real spawn: pop.value
    visible: spawn > 0.001

    PopFx {
        id: pop
    }

    // The zoom pivot is the board's drawn centre, so the cluster grows out of
    // the row it is anchored on (the board keeps its own fit scale).
    transform: Scale {
        origin.x: board.x + board.drawnWidth / 2
        origin.y: board.y + board.drawnHeight / 2
        xScale: root.spawn
        yScale: root.spawn
    }

    function arm(item, cfg) {
        if (!available || dismissed || !item || !cfg)
            return;
        targetItem = item;
        config = cfg;
        sticky = cfg.sticky === true;
        aimed = "";
        ratingValue = (cfg.south && typeof cfg.south.rate === "function")
            ? (Number(cfg.south.rating) || 0) : 0;
        ratingTouched = false;
        ratingDirty = false;
        armed = true;
        pop.play();
    }

    function cancel() {
        ratingCommit.stop();
        armed = false;
        aimed = "";
        ratingTouched = false;
        ratingDirty = false;
        sticky = false;
        pop.collapse();
    }

    /// UI-60: Shift-J/K over the aimed favourite slot steps its rating
    /// (`delta` -1/+1, clamped 0..5). Returns false when the aim is not a
    /// rating slot.
    function adjustRating(delta) {
        if (!armed || aimed !== "south" || !config || !config.south
                || typeof config.south.rate !== "function")
            return false;
        ratingValue = Math.max(0, Math.min(5, ratingValue + delta));
        // Local-first: the anchored surface.s star array + count repaint on
        // this step; `rate` only persists the level.
        if (config.south.preview)
            config.south.preview(ratingValue);
        ratingTouched = true;
        ratingDirty = true;
        ratingCommit.restart();
        // UI-60 "bright": only an increase earns the burst. The stars leave
        // the heart's centre — the south tile is aimed, so it is laid out.
        if (delta > 0) {
            var p = southBox.mapToItem(root, southBox.width / 2, southBox.height / 2);
            root.ratingStepped(p.x, p.y);
        }
        return true;
    }

    /// Sends the pending rating step (the idle timer's landing pad and the
    /// Alt release's flush).
    function commitRating() {
        ratingCommit.stop();
        if (!ratingDirty || !config || !config.south
                || typeof config.south.rate !== "function")
            return;
        ratingDirty = false;
        config.south.rate(ratingValue);
    }

    /// IPC `quick_cancel`: retire the overlay for the rest of this Alt hold.
    function dismiss() {
        dismissed = true;
        cancel();
    }

    function opposite(direction) {
        if (direction === "north")
            return "south";
        if (direction === "south")
            return "north";
        if (direction === "east")
            return "west";
        return "east";
    }

    /// Keyboard aiming (h/j/k/l, arrows). The center is a real neutral state:
    /// aiming toward the quadrant opposite the current one steps back to
    /// neutral first, the next press continues to that side. An open or
    /// disabled slot is never aimed — a press toward it lands on neutral.
    function aim(direction) {
        if (!armed)
            return;
        var next = aimed === opposite(direction) ? "" : direction;
        if (next.length > 0 && (!config || !config[next] || config[next].disabled === true))
            next = "";
        aimed = next;
    }

    function confirm() {
        // A rating step closes out with its own call: the Alt release must
        // not also toggle the favourite underneath it.
        var ratingAim = ratingTouched && aimed === "south";
        commitRating();
        var slot = aimed.length > 0 && config ? config[aimed] : null;
        if (slot && slot.invoke && !ratingAim)
            slot.invoke();
        cancel();
    }

    // Alt release confirms; a modal context or a half-typed sequence retires
    // the overlay instead (the roadmap hard rules).
    Connections {
        target: Backend
        function onAlt_held_changed() {
            if (!Backend.altHeld) {
                root.dismissed = false;
                // Sticky menus fire on Enter, not on an Alt release.
                if (!root.sticky)
                    root.confirm();
            }
        }
        function onKey_pending_changed() {
            if (Backend.keyPending)
                root.cancel();
        }
        // ARC-I: the search arms its own column while open; opening it
        // otherwise retires a list-armed overlay.
        function onSearch_open_changed() {
            if (Backend.searchOpen)
                root.cancel();
        }
    }

    // While armed, swallow clicks so an errant press cannot activate the row
    // underneath mid-gesture; a click that no tile claims cancels the menu
    // (the mouse equivalent of releasing over neutral). Hover passes through
    // (no hoverEnabled).
    TapHandler {
        enabled: root.armed
        acceptedButtons: Qt.LeftButton
        onTapped: root.cancel()
    }

    // ARC-UI-1: one parent rectangle owns the cluster — the margin between the
    // frame and each tile, and the option scale. The frame is a square built
    // on the target's larger side, so the four tiles (edge midpoints) are
    // equidistant from the neutral centre and h/j/k/l travel the same
    // distance; the neutral mark sits at its centre.
    Rectangle {
        radius: Theme.radius
        id: board

        /// ARC-UI-1 scale ramp: 1.7 was the pre-ramp value; each review pass
        /// trimmed the cap (x0.8, then 60% of that) and the ramp keeps easing
        /// down with the window itself.
        readonly property real maxOptionScale: 1.7 * 0.8 * 0.6
        readonly property real minOptionScale: 0.5

        /// Scale-independent tile metrics: `unitFont` sizes the open-slot
        /// "—" and `unitBoxSide` the square the verb icon fills; the ramp
        /// scales both. The tile side was halved per review (the squares
        /// were scaling too large).
        readonly property real unitFont: Math.max(11, root.height * 0.018)
        readonly property real unitBoxSide: root.height * 0.07
        readonly property real maxBoxSide: unitBoxSide * maxOptionScale

        /// The frame↔tile gap, grown so the four squares always clear each
        /// other (the old fixed margin let them overlap).
        readonly property real baseMargin: root.height * 0.02
        readonly property real margin: Math.max(baseMargin, maxBoxSide * 0.25,
            (maxBoxSide - Math.max(root.targetWidth, root.targetHeight)) / 2 + baseMargin)

        /// The no-cut range: the clamp starts 10 px before the window walls.
        readonly property real wallGap: 10
        readonly property rect safeRect: Qt.rect(
            root.windowRect.x + wallGap,
            root.windowRect.y + wallGap,
            Math.max(1, root.windowRect.width - 2 * wallGap),
            Math.max(1, root.windowRect.height - 2 * wallGap))

        /// The scale ramp: the cap on a large window, easing down with the
        /// window itself (the old fixed 1.7 was tuned for ~1280x720 and up).
        /// Overflow a wide target leaves is still caught by `fitScale` below.
        readonly property real rampWidth: 1280
        readonly property real rampHeight: 720
        readonly property real optionScale: Theme.clamp(
            maxOptionScale * Math.min(safeRect.width / rampWidth,
                                      safeRect.height / rampHeight),
            minOptionScale, maxOptionScale)

        /// The four tiles are squares of one side, placed on the edge
        /// midpoints of the board's square frame.
        readonly property real boxFont: unitFont * optionScale
        readonly property real boxSide: unitBoxSide * optionScale

        /// Fit: the board shrinks only when moving cannot keep it whole (it
        /// is wider or taller than the safe rect), so both edges can be seen.
        readonly property real fitScale: Math.min(1,
            safeRect.width / Math.max(1, width),
            safeRect.height / Math.max(1, height))
        readonly property real drawnWidth: width * fitScale
        readonly property real drawnHeight: height * fitScale

        /// The square frame: the target's larger side plus the margin on
        /// every side and a tile on each edge, so all four moves land the
        /// same distance from the centre. The interior gap (centre to a
        /// tile's inner edge) is capped at 3x the tile, so a wide target
        /// cannot fling the tiles far from the neutral (frame = 2 tiles +
        /// 6 tiles = 8 tiles at the cap).
        readonly property real frameMaxSide: 8 * boxSide
        readonly property real frameSide: Math.min(
            Math.max(root.targetWidth, root.targetHeight) + 2 * margin + 2 * boxSide,
            frameMaxSide)

        width: frameSide
        height: frameSide
        color: "transparent"
        transformOrigin: Item.TopLeft
        scale: fitScale
        // Centred on the target, then nudged back inside the 10 px safe rect:
        // cut by the right edge moves left, by the left moves right; when both
        // are exceeded the fit scale shrinks the whole cluster instead.
        x: Math.max(safeRect.x,
            Math.min(safeRect.x + safeRect.width - drawnWidth,
                     root.anchor.x - drawnWidth / 2))
        y: Math.max(safeRect.y,
            Math.min(safeRect.y + safeRect.height - drawnHeight,
                     root.anchor.y - drawnHeight / 2))

        component OptionBox: AcrylicPanel {
            id: box
            required property string direction

            readonly property var slot: root.config ? root.config[box.direction] : null
            readonly property string labelText: box.slot && box.slot.label ? box.slot.label : "—"
            readonly property string iconSource: box.slot && box.slot.icon ? box.slot.icon : ""
            readonly property bool isAimed: root.aimed === box.direction
            // VI-4: a verb that does not apply to the current selection is
            // shown gray and cannot be aimed.
            readonly property bool isDisabled: !!box.slot && box.slot.disabled === true
            // UI-60: the favourite slot carries a 0..=5 rating badge.
            readonly property bool ratingSlot: !!box.slot
                && typeof box.slot.rate === "function"
            readonly property int shownRating: root.ratingValue

            width: board.boxSide
            height: board.boxSide
            radius: Theme.radius
            backdrop: root.backdrop
            tint: box.isDisabled ? Theme.stateDisabledContainer
                : (box.isAimed ? Palette.primaryContainer : Palette.surfaceContainerHigh)
            opacity: box.isDisabled ? 0.7 : (!root.aimed || box.isAimed ? 1 : 0.55)
            scale: box.isAimed ? 1.3 : 1.0

            Behavior on tint {
                ColorAnimation {
                    duration: Theme.motion.micro
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }
            Behavior on opacity {
                NumberAnimation { duration: Theme.motion.micro }
            }
            Behavior on scale {
                NumberAnimation {
                    duration: Theme.motion.micro
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }

            // ARC-UI-1: the verb is its SVG glyph; an open slot keeps the
            // "—" text.
            Icon {
                anchors.centerIn: parent
                width: Math.min(parent.width, parent.height) * 0.55
                height: width
                visible: box.iconSource.length > 0
                source: box.iconSource
                symbolColor: box.isDisabled ? Theme.stateDisabledContent
                    : (box.isAimed ? Palette.onPrimaryContainer : Palette.onSurfaceVariant)
            }

            AppText {
                id: boxLabel
                anchors.centerIn: parent
                visible: box.iconSource.length === 0
                text: box.labelText
                color: box.isDisabled ? Theme.stateDisabledContent
                    : (box.isAimed ? Palette.onPrimaryContainer : Palette.onSurfaceVariant)
                font.family: "monospace"
                font.pixelSize: board.boxFont
            }

            // UI-60: the rating count at the favourite glyph's bottom-right.
            RatingBadge {
                visible: box.ratingSlot
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                anchors.margins: parent.width * 0.07
                width: parent.width * 0.448
                height: width
                rating: box.shownRating
            }

            // Mouse path: clicking a tile aims and fires it in one step.
            TapHandler {
                enabled: !box.isDisabled
                onTapped: {
                    root.aimed = box.direction;
                    root.confirm();
                }
            }
        }

        OptionBox {
            id: northBox
            direction: "north"
            anchors.top: parent.top
            anchors.horizontalCenter: parent.horizontalCenter
        }

        OptionBox {
            id: southBox
            direction: "south"
            anchors.bottom: parent.bottom
            anchors.horizontalCenter: parent.horizontalCenter
        }

        OptionBox {
            id: westBox
            direction: "west"
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
        }

        OptionBox {
            id: eastBox
            direction: "east"
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
        }

        // Center mark: the neutral aim (the four-way move glyph). Grows while
        // neutral so the state reads.
        Icon {
            anchors.centerIn: parent
            width: Math.max(16, root.height * 0.026)
            height: width
            source: "qrc:/app/icons/arrow-move-20-regular.svg"
            symbolColor: Palette.primary
            opacity: root.aimed === "" ? 1.0 : 0.6
            scale: root.aimed === "" ? 1.3 : 1.0

            Behavior on scale {
                NumberAnimation {
                    duration: Theme.motion.micro
                    easing.type: Easing.Bezier
                    easing.bezierCurve: Theme.ease.standard
                }
            }
            Behavior on opacity {
                NumberAnimation { duration: Theme.motion.micro }
            }
        }
    }
}
