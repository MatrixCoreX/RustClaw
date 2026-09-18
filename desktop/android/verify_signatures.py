"""Independently verify synthetic signatures pulled from Android instrumentation."""
import argparse
import json
from pathlib import Path
import sys
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec, utils

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'tests'))
from wallet_fixture import public_bytes

if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('signatures',type=Path)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    entries=json.loads(args.signatures.read_text())
    assert len(entries)==2
    kinds=[]
    for item in entries:
        raw=bytes.fromhex(item['signature'])
        assert len(raw)==64
        signature=utils.encode_dss_signature(int.from_bytes(raw[:32],'big'),int.from_bytes(raw[32:],'big'))
        key=ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256K1(),public_bytes(item['public_key']))
        key.verify(signature,item['payload'].encode(),ec.ECDSA(hashes.SHA256()))
        kinds.append(json.loads(item['payload'])['terms']['kind'])
    assert sorted(kinds)==['bancor_trade','transfer']
    result=dict(ok=True,signatures=len(entries),algorithm='secp256k1 ECDSA SHA-256',
                scope='freshly restored Android exported account',fixture_only=True)
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))
