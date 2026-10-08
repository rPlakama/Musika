# Where do we go now?

[X] -- Init QT;
[X] -- Init Rust;
[X] -- Init HTTP;
[] -- Bridge to Navidrome API
    -> Usrname/Passwd write using Sqlite; _[X]_
    -> Login Screen in QT; _[X]_
    -> Wireup qml.rs <--> main.rs <--> sqlite.rs <--> http.rs, objective: Functional Login Screen that can ping if password is wrong, notice, use ENC to write down the DB;
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
