CREATE TABLE file_hashes (
    path BLOB NOT NULL PRIMARY KEY,
    stamp BLOB NOT NULL CHECK(length(stamp) = 32),
    hash BLOB NOT NULL CHECK(length(hash) = 32)
);
