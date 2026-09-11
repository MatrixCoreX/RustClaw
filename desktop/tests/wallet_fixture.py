"""Owner protocol fixture: loopback only, real secp256k1 verification, no ledger funds."""
import copy
import hashlib
import json
import socket
import time
import uuid
from urllib.parse import parse_qs, urlparse
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec, utils

ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'

def public_bytes(encoded):
    n = 0
    for c in encoded:
        n = n * 58 + ALPHABET.index(c)
    raw = n.to_bytes(37, 'big')
    assert hashlib.new('ripemd160', raw[:33] + b'K1').digest()[:4] == raw[33:]
    return raw[:33]

class OwnerApi:
    def __init__(self):
        self.challenges = {}
        self.outcomes = {}
        self.verified = []
        self.public_reads = []
        self.tamper = None
        self.drop_response = False
        self.unsupported = False
        self.ledger = 'ledger-fixture-v1'
        self.node = 'https://ledger.example.test'

    def market_data(self, url):
        now = int(time.time())
        common = dict(schema_version=1, market_id='fixture-market', node_url=self.node)
        if '/candles' in url:
            interval = int(parse_qs(urlparse(url).query).get('interval_seconds', ['3600'])[0])
            end = now // interval * interval
            candles = [dict(bucket_start_unix=end-(20-i)*interval, bucket_end_unix=end-(19-i)*interval,
                open='2.0', high='2.1', low='1.9', close='2.0', aic_volume_units='100000000', aic_volume='1',
                usd_volume_units='200000000', usd_volume='2', trade_count=1, has_trades=True,
                liquidity_event_count=0, liquidity_usd_units='0', liquidity_usd='0') for i in range(20)]
            return dict(**common, status='bancor_candles', market_version=1, market_created_at_unix=now-86400*365,
                price_kind='pool_marginal_usd_per_aic', interval_seconds=interval,
                start_time_unix=candles[0]['bucket_start_unix'], end_time_unix=end,
                price_scale=1000000000000, price_decimal_places=12, candles=candles)
        if '/trades' in url:
            return dict(**common, status='bancor_market_trades', limit=100, trades=[])
        return dict(**common, status='open', aic_symbol='AIC', usd_symbol='USD', aic_scale=100000000,
            usd_scale=100000000, aic_reserve_units='100000000000', aic_reserve='1000',
            usd_reserve_units='200000000000', usd_reserve='2000', marginal_price_usd_per_aic='2',
            daily_marginal_price=dict(price_kind='pool_marginal_usd_per_aic', timezone='UTC',
                day_start_unix=now//86400*86400, open_usd_per_aic='2', high_usd_per_aic='2.1', low_usd_per_aic='1.9', change_percent='0', trade_count=20),
            min_trade_usd='1', min_trade_usd_units='100000000', min_trade_aic='1', min_trade_aic_units='100000000',
            minimum_fee_units='0', minimum_output_units='1', fee_bps=0, version=1, updated_at_unix=now)

    def handle(self, handler, body):
        def reply(data):
            return handler.send_data({'ok': True, 'data': data})
        if self.unsupported:
            return handler.send_data({'ok': False, 'error': 'unsupported'}, 404)
        path = handler.path.split('?')[0]
        if path.endswith('/capabilities'):
            service = parse_qs(urlparse(handler.path).query)['service'][0]
            return reply(dict(schema_version=1, protocol='asset_owner_v1', ledger_id=self.ledger,
                node_url=self.node, service=service, actions=['balances', 'history', 'operation_status',
                'transfer' if service == 'assets' else 'bancor_trade']))
        if path.endswith('/read/public'):
            assert 'signature' not in body and 'password' not in body
            terms = body['intent']
            assert terms['kind'] in ('balances', 'history', 'operation_status')
            self.public_reads.append(body)
            if terms['kind'] == 'operation_status':
                return reply(self.outcomes[terms['operation_id']])
            return reply(dict(account=body['account'], ledger_id=self.ledger, node_url=self.node,
                aic_balance_units='12345000000', usd_balance_units='9000000000',
                page=terms.get('page', 1), total_pages=2, records=[dict(operation_id=str(uuid.uuid5(uuid.NAMESPACE_URL, body['account'] + str(terms.get('page', 1)))), kind='transfer_in' if terms.get('page', 1)==1 else 'bancor_buy', asset='AIC', amount_units='100000000', counterparty=None, created_at_unix=int(time.time()))]))
        if path.endswith('/request'):
            terms = copy.deepcopy(body['intent'])
            if terms['kind'] == 'bancor_trade':
                quote = int(terms['input_units']) * 2
                terms.update(fee_units='0', quoted_output_units=str(quote),
                    min_output_units=str(quote * (10000 - terms['slippage_bps']) // 10000))
            elif terms['kind'] == 'transfer':
                terms['fee_units'] = '0'
            payload = {k: body[k] for k in ['schema_version', 'protocol', 'ledger_id', 'node_url', 'service', 'account', 'operation_id']}
            payload.update(challenge_id=str(uuid.uuid4()), nonce=uuid.uuid4().hex + uuid.uuid4().hex,
                expires_at_unix=int(time.time()) + 120, terms=terms)
            if self.tamper:
                payload['terms'][self.tamper[0]] = self.tamper[1]
            raw = json.dumps(payload, separators=(',', ':'), ensure_ascii=False)
            self.challenges[payload['challenge_id']] = (payload, raw)
            return reply({'signing_payload': raw})
        if path.endswith('/verify'):
            payload, raw = self.challenges.pop(body['challenge_id'])
            assert payload['expires_at_unix'] > time.time()
            for key in ['account', 'operation_id', 'ledger_id', 'node_url', 'service', 'protocol', 'schema_version']:
                assert body[key] == payload[key], key
            signature = bytes.fromhex(body['signature'])
            assert len(signature) == 64
            r, s = int.from_bytes(signature[:32]), int.from_bytes(signature[32:])
            key = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256K1(), public_bytes(body['account']))
            key.verify(utils.encode_dss_signature(r, s), raw.encode(), ec.ECDSA(hashes.SHA256()))
            self.verified.append(payload)
            terms = payload['terms']
            if terms['kind'] == 'operation_status':
                return reply(self.outcomes[terms['operation_id']])
            if terms['kind'] in ('balances', 'history'):
                return reply(dict(account=body['account'], ledger_id=self.ledger, node_url=self.node,
                    aic_balance_units='12345000000', usd_balance_units='9000000000',
                    page=terms.get('page', 1), total_pages=2, records=[dict(operation_id=str(uuid.uuid5(uuid.NAMESPACE_URL, body['account'] + str(terms.get('page', 1)))), kind='transfer_in' if terms.get('page', 1)==1 else 'bancor_buy', asset='AIC', amount_units='100000000', counterparty=None, created_at_unix=int(time.time()))]))
            outcome = dict(operation_id=body['operation_id'], account=body['account'], ledger_id=self.ledger,
                status='succeeded', receipt_id='fixture-receipt-' + body['operation_id'])
            self.outcomes[body['operation_id']] = outcome
            if self.drop_response:
                self.drop_response = False
                handler.connection.shutdown(socket.SHUT_RDWR)
                handler.connection.close()
                return
            return reply(outcome)
        return handler.send_data({'ok': False, 'error': 'unknown'}, 404)
