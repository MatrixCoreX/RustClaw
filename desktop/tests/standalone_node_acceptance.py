"""Connection changes revoke outstanding confirmations; optional real-node reads."""
import json
import os
import time
from http.client import RemoteDisconnected
from fixture_server import Fixture
from standalone_fixture import StandaloneApi


def run(native,execute,switch,main,wallet,account,second,fixture,node,output,passed):
    other = Fixture(output/'second-node')
    other.standalone_api = StandaloneApi(other.local_origin,second['public_key'])
    try:
        connection = native('wallet_connect_node',{'nodeId':node['id']})
        native('wallet_select',{'accountId':account['id']})
        new_node = native('wallet_add_node',{'origin':other.local_origin})
        fixture.standalone_api.capabilities_delay = .3
        other.standalone_api.capabilities_delay = .02
        preferred = native('wallet_prefer_node')
        assert preferred['automatic'] and preferred['node']['id'] == new_node['id']
        assert preferred['response_ms'] >= 20
        assert native('wallet_nodes')['ledger_id'] == fixture.standalone_api.ledger
        # A faster different ledger is excluded, even after it was once selected.
        other.standalone_api.ledger = 'different-fixture-ledger'
        same_ledger = native('wallet_prefer_node')
        assert same_ledger['node']['id'] == node['id']
        # A failed selection must retain the current session and saved node.
        fixture.standalone_api.unsupported = True
        other.standalone_api.unsupported = True
        native('wallet_prefer_node',error='wallet_node_no_healthy')
        assert native('wallet_nodes')['selected'] == node['id']
        fixture.standalone_api.unsupported = False
        other.standalone_api.unsupported = False
        assert native('wallet_read',{'sessionId':same_ledger['id'],'accountId':account['id'],'service':'assets','page':None})['node_url'] == fixture.local_origin
        fixture.standalone_api.capabilities_delay = 0
        other.standalone_api.ledger = fixture.standalone_api.ledger
        connection = native('wallet_connect_node',{'nodeId':node['id']})
        assert not connection['automatic'] and connection['node']['id'] == node['id']
        passed('node preference chooses the faster valid node, excludes another ledger, preserves session on all failures and retains explicit manual selection')
        command = {'sessionId':connection['id'],'accountId':account['id'],'service':'assets',
            'intent':dict(kind='transfer',asset='AIC',amount_units='100000000',recipient=second['public_key'],memo='fixture cancellation',max_fee_bps=0)}
        try:
            execute("window.__TAURI_INTERNALS__.invoke('wallet_prepare',arguments[0]).then(id=>window.testPending=id).catch(e=>window.testError=String(e));return true;",[command])
        except RemoteDisconnected: pass
        time.sleep(1)
        switch(wallet)
        request = native('wallet_pending')
        assert request
        switch(main)
        native('wallet_prefer_node',error='wallet_confirmation_pending')
        switch(wallet)
        assert native('wallet_pending')['payload']['operation_id'] == request['payload']['operation_id']
        switch(main)
        updated = native('wallet_connect_node',{'nodeId':new_node['id']})
        assert updated['node']['origin'] == other.local_origin and updated['id'] != connection['id']
        native('wallet_read',{'sessionId':connection['id'],'accountId':account['id'],'service':'assets','page':None},'stale_connection')
        assert native('wallet_read',{'sessionId':updated['id'],'accountId':account['id'],'service':'assets','page':None})['node_url'] == other.local_origin
        switch(wallet)
        assert native('wallet_pending') is None and not native('wallet_status')['unlocked']
        native('wallet_confirm',{'operationId':request['payload']['operation_id'],'password':'fixture-vault-password'},'wallet_confirmation_missing')
        assert not other.standalone_api.verified
        switch(main)
        passed('changing to another asset node cancels pending confirmation, locks wallet and rejects stale connection without a financial write')
        # A lost reply leaves a durable unknown result; preference must not hide
        # that operation on another node or submit it again.
        command['sessionId'] = updated['id']
        other.standalone_api.drop_response = True
        try:
            execute("window.__TAURI_INTERNALS__.invoke('wallet_prepare',arguments[0]).then(id=>window.testPending=id).catch(e=>window.testError=String(e));return true;",[command])
        except RemoteDisconnected: pass
        time.sleep(1)
        switch(wallet)
        pending = native('wallet_pending')
        assert pending
        execute("document.querySelector('[data-desktop-language-toggle]').click();return true;")
        for _ in range(30):
            if execute("return document.body.textContent.includes('Recipient account')"): break
            time.sleep(.1)
        assert execute("return document.body.textContent.includes('Recipient account')")
        assert native('wallet_pending')['payload'] == pending['payload']
        execute("document.querySelector('[data-desktop-language-toggle]').click();return true;")
        time.sleep(2.1)
        native('wallet_confirm',{'operationId':pending['payload']['operation_id'],'password':'fixture-vault-password'},'wallet_outcome_unknown')
        switch(main)
        native('wallet_prefer_node',error='wallet_unresolved_operation')
        assert native('wallet_nodes')['selected'] == new_node['id']
        assert native('wallet_check_operation',{'sessionId':updated['id'],'accountId':account['id'],'service':'assets','operationId':pending['payload']['operation_id']})['status'] == 'succeeded'
        assert len(other.standalone_api.verified) == 1
        passed('node preference preserves pending confirmation and unknown submitted result; outcome is checked on original node with no resubmission')
        native('wallet_disconnect_node')
        if os.environ.get('DESKTOP_TEST_PUBLIC_ASSET_NODES') == '1':
            # Temporary unfunded fixture account only. No operation requests or signatures.
            evidence=[]
            for origin in ['https://api-1.matrixai.one','https://api-2.matrixai.one']:
                public_node=native('wallet_add_node',{'origin':origin})
                public_session=native('wallet_connect_node',{'nodeId':public_node['id']})
                assert native('current_session') is None
                read=native('wallet_read',{'sessionId':public_session['id'],'accountId':account['id'],'service':'assets','page':None})
                assert read['account']==account['public_key'] and read['node_url']==origin
                market=native('wallet_market_read',{'sessionId':public_session['id'],'path':'/v1/nni/bancor/market'})
                history=native('wallet_market_read',{'sessionId':public_session['id'],'path':f'/v1/nni/assets/transfers?owner_pubkey={account["public_key"]}&limit=100&page=1&source=all&direction=all'})
                candles=native('wallet_market_read',{'sessionId':public_session['id'],'path':'/v1/nni/bancor/candles?interval_seconds=300&limit=30'})
                evidence.append({'origin':origin,'ledger_id':read['ledger_id'],'account':read['account'],
                    'balances':{'AIC':read['aic_balance_units'],'USD':read['usd_balance_units']},
                    'market_ok':market['ok'],'history_ok':history['ok'],'candles_ok':candles['ok']})
                native('wallet_disconnect_node')
            (output/'public-node-native-readonly.json').write_text(json.dumps(evidence,indent=2)+'\n')
            passed('both deployed HTTPS nodes: native certificate verification, owner capabilities, public balances, full history, market and pool-price candles without device login or signing')
    finally:
        other.close()
