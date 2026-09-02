import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import { NniPublicKeyDisplay } from "../components/NniPublicKeyDisplay";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const rawPublicKey =
  "2b9c9d84fa15f4e178ce58d0a40a9f5e150e9c502e689a24d0c0f221337870c" +
  "726f0e463d730a75401c425bfde0db0c442e314027d83885a84c535eaa35460a0";

test("labels public-key formats as raw and Base58 instead of compact", async () => {
  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(<NniPublicKeyDisplay value={rawPublicKey} t={(zh) => zh} />);
  });

  try {
    const switchButton = renderer!.root.findByType("button");
    assert.equal(switchButton.props.title, "切换为原始十六进制公钥");

    await act(async () => switchButton.props.onClick());
    const encodedButton = renderer!.root.findByType("button");
    assert.equal(encodedButton.props.title, "切换为 Base58 编码公钥");
    const text = encodedButton.children.filter((child) => typeof child === "string").join("");
    assert.equal(text, "Base58");
    assert.doesNotMatch(text, /紧凑|Compact/);
  } finally {
    await act(async () => renderer?.unmount());
  }
});

test("compact copy button copies the complete displayed public key", async () => {
  const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  let copied = "";
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      clipboard: {
        writeText: async (value: string) => {
          copied = value;
        },
      },
    },
  });

  let renderer: ReactTestRenderer | null = null;
  try {
    await act(async () => {
      renderer = create(
        <NniPublicKeyDisplay
          value={rawPublicKey}
          t={(zh) => zh}
          allowFormatSwitch={false}
          copyButton="compact"
        />,
      );
    });

    const copyButton = renderer!.root.findByType("button");
    const displayedPublicKey = renderer!.root.findByType("code").children.join("");
    assert.equal(copyButton.props["aria-label"], "复制完整公钥");
    await act(async () => {
      copyButton.props.onClick();
      await Promise.resolve();
    });
    assert.equal(copied, displayedPublicKey);
    assert.equal(renderer!.root.findByType("button").props.title, "已复制完整公钥");
  } finally {
    await act(async () => renderer?.unmount());
    if (originalNavigator) {
      Object.defineProperty(globalThis, "navigator", originalNavigator);
    } else {
      delete (globalThis as typeof globalThis & { navigator?: unknown }).navigator;
    }
  }
});
