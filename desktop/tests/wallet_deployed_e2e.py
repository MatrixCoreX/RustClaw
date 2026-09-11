"""Opt-in native-to-deployed gateway acceptance. No real funds or writes accepted."""
import json
import os
from pathlib import Path
import stat


def run(native, account, recipient, output):
    origin = os.environ.get('OWNER_DEPLOYED_ORIGIN')
    if not origin:
        return False
    credential = Path(os.environ['OWNER_DEPLOYED_KEY_FILE'])
    assert stat.S_IMODE(credential.stat().st_mode) == 0o600
    key = credential.read_text().strip()
    assert key
    connection = ({'kind': 'local', 'origin': origin} if origin.startswith('http:')
                  else {'kind': 'https', 'origin': origin})
    native('disconnect_device')
    profile = native('add_profile', {'alias': 'Deployed owner acceptance', 'connection': connection})
    session = native('connect_device', {'profileId': profile['id'], 'sshSecret': ''})['id']
    native('login', {'sessionId': session, 'input': {'mode': 'key', 'username': '', 'secret': key}, 'remember': False})
    del key
    native('wallet_select', {'accountId': account['id']})
    assert not native('wallet_status')['unlocked']
    evidence = []
    for service in ('assets', 'bancor'):
        cap = native('wallet_capabilities', {'sessionId': session, 'service': service})
        base = {'sessionId': session, 'accountId': account['id'], 'service': service}
        for page in (None, 1):
            data = native('wallet_read', {**base, 'page': page})
            assert data['account'] == account['public_key']
            assert data['ledger_id'] == cap['ledger_id'] and data['node_url'] == cap['node_url']
            assert data['aic_balance_units'] == data['usd_balance_units'] == '0'
            assert not data['records']
        intent = ({'kind': 'transfer', 'asset': 'USD', 'amount_units': '100000000',
                   'recipient': recipient['public_key'], 'memo': 'unfunded acceptance', 'max_fee_bps': 0}
                  if service == 'assets' else {'kind': 'bancor_trade', 'side': 'buy',
                   'input_units': '100000000', 'slippage_bps': 300, 'max_fee_bps': 5000})
        # No signature or real balance: Core must reject before confirmation.
        native('wallet_prepare', {**base, 'intent': intent}, 'wallet_insufficient_balance')
        assert not native('wallet_operations', {'sessionId': session, 'accountId': account['id']})
        evidence.append({'service': service, 'account': account['public_key'], 'node_url': cap['node_url'], 'ledger_id': cap['ledger_id'],
                         'zero_reads': True, 'unfunded_write_rejected': True})
    assert evidence[0]['ledger_id'] == evidence[1]['ledger_id']
    (output / 'deployed-acceptance.json').write_text(json.dumps(evidence, indent=2))
    return True
