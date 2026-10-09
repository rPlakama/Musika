# Where do we go now?

[X] -- Init QT;
[X] -- Init Rust;
[X] -- Init HTTP;
[] -- Bridge to Navidrome API
    -> Usrname/Passwd write using Sqlite; _[X]_
    -> Login Screen in QT; _[X]_
    -> Wireup qml.rs <--> main.rs [X] <--> sqlite.rs <--> http.rs, objective: Functional Login Screen that can ping if password is wrong, notice, use ENC to write down the DB;
    -> Elaborating current ideia:

    User types into QML fields → set_username/set_password/set_server_url fire → fields update on Backend, still on the Qt thread.
    User clicks login → QML invokes the login slot → Qt thread, synchronous.
    login() clones pool, rt, and the three String fields out of self (no more borrow of self needed) and calls rt.spawn(async move { ... }).
    login() returns immediately — Qt thread is free, UI stays responsive.
    On a tokio worker thread, authenticate() runs the actual query, .awaits on I/O without blocking anything.
    When it resolves, the result needs to cross back to the Qt thread via whatever safe mechanism qtbridge provides, and only then does something call self.login_succeeded() or self.login_failed() to update the UI.

[] -- Local Playback thought streaming using FFMPEG or MPV, maybe.;
[] -- Init Playable System
[] -- Initial Design
...

# Features that I want

[] -- Sync (Indexing the Navidrome with QLite?)
[] -- Lyrics
[] -- Download Caching
[] -- Cover Caching
[] -- Auto-Sync

# Per Design

DB1 = usrname stuff
DB2 = index of content
