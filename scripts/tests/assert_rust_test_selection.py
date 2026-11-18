#!/usr/bin/env python3
"""Reject empty or incomplete stable-Rust --list output before a focused test."""
import argparse
import json
import re
import sys


def selection(text, prefix, required=()):
    names=re.findall(r'^([A-Za-z0-9_:]+): test$', text, re.M)
    if len(names)!=len(set(names)):
        raise ValueError('rust_test_selection_duplicate')
    selected=sorted(name for name in names if name.startswith(prefix))
    expected={prefix+name for name in required}
    if not selected or not expected.issubset(selected):
        raise ValueError('rust_test_selection_missing')
    return dict(schema_version=1,prefix=prefix,test_count=len(selected),tests=selected)


def self_test():
    prefix='fixture::tests::'
    result=selection('fixture::tests::first: test\nfixture::tests::second: test\n2 tests, 0 benchmarks\n',prefix,['first','second'])
    assert result['test_count']==2
    for text,required in [
        ('0 tests, 0 benchmarks\n',[]),
        ('other::tests::first: test\n',[]),
        ('warning: fixture::tests::first: test\n',[]),
        ('fixture::tests::first: benchmark\n',[]),
        ('fixture::tests::first: test\n',['second']),
        ('fixture::tests::first: test\nfixture::tests::first: test\n',[]),
    ]:
        try:
            selection(text,prefix,required)
        except ValueError:
            continue
        raise AssertionError('invalid_selection_accepted')
    print('RUST_TEST_SELECTION_SELF_TEST ok cases=7')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test',action='store_true')
    parser.add_argument('--prefix')
    parser.add_argument('--require',action='append',default=[])
    args=parser.parse_args()
    if args.self_test:
        self_test()
        return
    if not args.prefix:
        parser.error('--prefix is required')
    try:
        result=selection(sys.stdin.read(),args.prefix,args.require)
    except ValueError as error:
        parser.exit(1,str(error)+'\n')
    print(json.dumps(result,sort_keys=True))


if __name__=='__main__':
    main()
