-- PATH-10: the Songs list and library queue sort on this order; the index
-- removes the whole-table sort from every page/queue build.
CREATE INDEX IF NOT EXISTS idx_songs_order
    ON songs (artist, album, disc_number, track, title);
