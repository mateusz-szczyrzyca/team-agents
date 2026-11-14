-- Exhausting an AUTOINCREMENT key makes SQLite return SQLITE_FULL without filling the host disk.
CREATE TABLE history_fault_full(id INTEGER PRIMARY KEY AUTOINCREMENT) STRICT;
INSERT INTO history_fault_full(id) VALUES(9223372036854775807);
CREATE TRIGGER exhaust_history_steps BEFORE INSERT ON steps
BEGIN
    INSERT INTO history_fault_full(id) VALUES(NULL);
END;
