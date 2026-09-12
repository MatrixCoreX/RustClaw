"""Public owner-node fixture; no device login, no real funds, real K1 verification."""
import re
import time
from urllib.parse import parse_qs, urlparse
from wallet_fixture import OwnerApi


class StandaloneApi(OwnerApi):
    def __init__(self, origin, partner):
        super().__init__()
        self.node = origin
        self.partner = partner
        self.contexts = {}
        self.requests = []
        self.capabilities_delay = 0

    def serve(self, handler, body):
        path = urlparse(handler.path).path
        if not path.startswith('/v1/nni/server/'):
            return False
        assert not handler.headers.get('Cookie')
        assert not handler.headers.get('Authorization')
        self.requests.append((handler.command, handler.path))
        if '/assets/owner/' in path:
            if path.endswith('/capabilities'):
                time.sleep(self.capabilities_delay)
            binding = handler.headers.get('x-agent-owner-context', '')
            assert re.fullmatch('[0-9a-f]{64}', binding)
            if path.endswith('/operations/request'):
                self.contexts[body['operation_id']] = binding
            if path.endswith('/operations/verify'):
                assert self.contexts[body['operation_id']] == binding
            self.handle(handler, body)
        elif path in ('/v1/nni/server/bancor/market', '/v1/nni/server/bancor/candles', '/v1/nni/server/bancor/trades'):
            if path.endswith('/candles'):
                assert parse_qs(urlparse(handler.path).query)['price_kind'] == ['pool_marginal_usd_per_aic']
            handler.send_data({'ok': True, 'data': self.market_data(handler.path)})
        elif path == '/v1/nni/server/explorer/transactions':
            query = parse_qs(urlparse(handler.path).query)
            owner, page = query['address'][0], int(query['page'][0])
            cls = query.get('transaction_class', [None])[0]
            direction = query.get('direction', [None])[0]
            rows = []
            for index in range((page-1)*100, min(page*100,101)):
                sender, recipient = (owner, self.partner) if direction == 'outgoing' else (self.partner, owner)
                kind = {'market_trade':'bancor_buy','system_issuance':'admin_usd_credit'}.get(cls,'asset_transfer')
                rows.append(dict(transaction_id=f'fixture-{kind}-{index}', transaction_kind=kind,
                    transaction_class=cls or 'peer_transfer', created_at_unix=int(time.time())-index,
                    memo='fixture memo', flows=[dict(flow_index=0,asset='AIC',amount_units='100000000',amount='1.00000000',
                        **{'from':dict(account_kind='asset_owner',address=sender),'to':dict(account_kind='asset_owner',address=recipient)})]))
            handler.send_data({'ok':True,'data':dict(schema_version=1,status='explorer_transactions',page=page,
                per_page=100,total=101,total_pages=2,filter=dict(address=owner,transaction_class=cls,direction=direction),transactions=rows)})
        else:
            handler.send_data({'ok':False,'error':'unsupported'},404)
        return True
