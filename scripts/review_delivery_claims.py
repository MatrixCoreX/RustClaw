"""List unresolved channel dispatches and record an operator's independent confirmation."""
import argparse
import hashlib
import json
import sqlite3
import time
from pathlib import Path

CLAIMS = """
SELECT c.idempotency_key,c.delivery_id,c.channel,c.state,c.attempt_count,
       c.updated_at_ts,c.lease_expires_at_ts,r.status AS receipt_status,
       r.receipt_digest
FROM channel_delivery_dispatch_claims c LEFT JOIN channel_delivery_receipts r
ON r.idempotency_key=c.idempotency_key AND r.delivery_id=c.delivery_id AND r.channel=c.channel
WHERE (c.state='query_required' OR (c.state='dispatching' AND c.lease_expires_at_ts<=?))
  AND (r.status IS NULL OR r.status NOT IN ('accepted','delivered','read'))
ORDER BY c.updated_at_ts,c.idempotency_key
"""
AUDIT = """CREATE TABLE IF NOT EXISTS channel_delivery_operator_reviews (
    delivery_id TEXT NOT NULL, snapshot_digest TEXT NOT NULL,
    evidence_ref TEXT NOT NULL, reviewed_at_ts INTEGER NOT NULL,
    PRIMARY KEY(delivery_id,snapshot_digest)
)"""

def inventory(db, now):
    reviewed = {}
    if db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='channel_delivery_operator_reviews'").fetchone():
        reviewed = dict(db.execute("SELECT snapshot_digest,reviewed_at_ts FROM channel_delivery_operator_reviews"))
    records = []
    for row in db.execute(CLAIMS, (now,)):
        item = dict(row)
        digest = hashlib.sha256(json.dumps(item, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        records.append({**item,'snapshot_digest':digest,
                        'review_status':'operator_confirmed' if digest in reviewed else 'unconfirmed'})
    return records

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('database', type=Path)
    parser.add_argument('--confirm-received', metavar='DELIVERY_ID')
    parser.add_argument('--snapshot-digest')
    parser.add_argument('--evidence-ref', help='Reference to independent recipient confirmation')
    args = parser.parse_args()
    if args.confirm_received and not (args.evidence_ref and args.evidence_ref.strip() and args.snapshot_digest):
        parser.error('confirmation needs --snapshot-digest and --evidence-ref')
    mode = 'rw' if args.confirm_received else 'ro'
    db = sqlite3.connect(args.database.resolve().as_uri()+'?mode='+mode, uri=True)
    db.row_factory = sqlite3.Row
    now = int(time.time())
    with db:
        if args.confirm_received:
            db.execute('BEGIN IMMEDIATE')
        items = inventory(db, now)
        if args.confirm_received:
            entry = next((i for i in items if i['delivery_id']==args.confirm_received and i['snapshot_digest']==args.snapshot_digest),None)
            if entry is None:
                parser.error('claim changed or already has a final receipt')
            db.execute(AUDIT)
            db.execute('INSERT OR IGNORE INTO channel_delivery_operator_reviews VALUES(?,?,?,?)',
                       (entry['delivery_id'],entry['snapshot_digest'],args.evidence_ref.strip(),now))
            items = [i for i in inventory(db,now) if i['delivery_id']==args.confirm_received]
    print(json.dumps(items,ensure_ascii=False,indent=2))
    db.close()

if __name__=='__main__':
    main()
