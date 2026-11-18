"""Build-free constraints of runtime-owned memory import staging."""
from pathlib import Path
import sqlite3
import unittest


SQL = (Path(__file__).resolve().parents[2] / 'migrations/032_memory_import_uploads.sql').read_text()


class UploadSchemaTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.db.execute('PRAGMA foreign_keys=ON')
        self.db.execute('CREATE TABLE principals(principal_id TEXT PRIMARY KEY)')
        self.db.execute("INSERT INTO principals VALUES('owner')")
        self.db.executescript(SQL)

    def tearDown(self):
        self.db.close()

    def create(self, total=4, received=0, chunk=2, revision=1, state='uploading', token=None, lease=None):
        self.db.execute("""INSERT INTO memory_import_uploads(upload_id,principal_id,file_digest,total_bytes,
            received_bytes,chunk_bytes,state,revision,parser_token,parser_lease_until_ts,created_at_ts,updated_at_ts)
            VALUES('upload','owner','sha256:fixture',?,?,?,?,?,?,?,100,100)""",
            [total,received,chunk,state,revision,token,lease])

    def test_invalid_sizes_and_half_leases_are_rejected(self):
        for changes in [dict(total=0),dict(received=-1),dict(received=5),dict(chunk=0),dict(revision=0),
                        dict(state='parsing'),dict(state='parsing',token='token'),
                        dict(token='token'),dict(lease=101)]:
            with self.subTest(changes=changes),self.assertRaises(sqlite3.IntegrityError):
                self.create(**changes)
            self.assertEqual(self.db.execute('SELECT COUNT(*) FROM memory_import_uploads').fetchone(),(0,))

    def test_foreign_owner_and_unknown_states_are_rejected(self):
        with self.assertRaises(sqlite3.IntegrityError):
            self.create(state='unknown')
        self.db.execute("DELETE FROM principals WHERE principal_id='owner'")
        with self.assertRaises(sqlite3.IntegrityError):
            self.create()

    def test_cancelled_and_completed_files_do_not_keep_active_unique_binding(self):
        self.create()
        command="""INSERT INTO memory_import_uploads(upload_id,principal_id,file_digest,total_bytes,
            received_bytes,chunk_bytes,state,revision,created_at_ts,updated_at_ts)
            VALUES('other','owner','sha256:fixture',4,0,2,'uploading',1,100,100)"""
        with self.assertRaises(sqlite3.IntegrityError):
            self.db.execute(command)
        self.db.execute("UPDATE memory_import_uploads SET state='cancelled' WHERE upload_id='upload'")
        self.db.execute(command)
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM memory_import_uploads').fetchone(),(2,))

    def test_chunk_count_body_integrity_and_receipt_only_rows(self):
        self.create()
        command="INSERT INTO memory_import_upload_chunks VALUES('upload',?,?,?,?)"
        for values in [(0,0,'digest',b''),(-1,2,'digest',b'ab'),(0,1,'digest',b'ab')]:
            with self.subTest(values=values),self.assertRaises(sqlite3.IntegrityError):
                self.db.execute(command,values)
        self.db.execute(command,[0,2,'digest',b'ab'])
        self.db.execute("UPDATE memory_import_upload_chunks SET content=NULL")
        self.assertEqual(self.db.execute('SELECT byte_count,content_digest,content FROM memory_import_upload_chunks').fetchone(),(2,'digest',None))

    def test_cascade_removes_staging_not_canonical_tables(self):
        self.create()
        self.db.execute("INSERT INTO memory_import_upload_chunks VALUES('upload',0,2,'digest',?)",[b'ab'])
        self.db.execute("INSERT INTO memory_import_upload_items VALUES('upload',0,'{}')")
        self.db.execute('CREATE TABLE memory_facts(memory_id TEXT)')
        self.db.execute("INSERT INTO memory_facts VALUES('canonical-witness')")
        self.db.execute("DELETE FROM memory_import_uploads WHERE upload_id='upload'")
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM memory_import_upload_chunks').fetchone(),(0,))
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM memory_import_upload_items').fetchone(),(0,))
        self.assertEqual(self.db.execute('SELECT * FROM memory_facts').fetchall(),[('canonical-witness',)])


if __name__ == '__main__':
    unittest.main()
