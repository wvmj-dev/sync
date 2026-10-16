import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RunStatus } from "../api/types";
import { formatBytes, formatRelative, plural, runStatusLabel, runTone } from "./format";

describe("formatBytes", () => {
  it("usa a unidade binária com uma casa", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1024)).toBe("1.0 KiB");
    expect(formatBytes(18_400_000)).toBe("17.5 MiB");
    expect(formatBytes(2_100_000_000)).toBe("2.0 GiB");
  });
});

describe("formatRelative", () => {
  const now = new Date("2026-10-16T12:00:00Z");
  const hours = (h: number) => new Date(now.getTime() + h * 3_600_000).toISOString();

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(now);
  });
  afterEach(() => vi.useRealTimers());

  it("fala do passado e do futuro em pt-BR", () => {
    expect(formatRelative(hours(-15))).toBe("há 15 horas");
    expect(formatRelative(hours(9))).toBe("em 9 horas");
    expect(formatRelative(hours(-0.2))).toBe("há 12 minutos");
    expect(formatRelative(hours(-48))).toBe("anteontem");
  });

  it("menos de um minuto é agora; sem data, um traço", () => {
    expect(formatRelative(new Date(now.getTime() - 30_000).toISOString())).toBe("agora");
    expect(formatRelative(null)).toBe("—");
  });
});

describe("plural", () => {
  it("concorda com a quantidade", () => {
    expect(plural(1, "item", "itens")).toBe("1 item");
    expect(plural(0, "item", "itens")).toBe("0 itens");
    expect(plural(42, "item", "itens")).toBe("42 itens");
  });
});

describe("resultado de execução", () => {
  it("todo status tem rótulo e tom", () => {
    const statuses = Object.keys(runStatusLabel) as RunStatus[];
    expect(statuses).toContain("postponed");
    for (const s of statuses) expect(runStatusLabel[s]).not.toBe("");
    // RN-84: adiada pede atenção, sem alarme; RN-90: descartada é neutra.
    expect(runTone("postponed")).toBe("warn");
    expect(runTone("discarded")).toBe("neutral");
    expect(runTone("failed")).toBe("danger");
    expect(runTone("success")).toBe("ok");
  });
});
