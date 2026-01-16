import { describe, test, expect } from "bun:test";
import { parseCommands } from "./parse-commands";

describe("parseCommands", () => {
  describe("basic single commands", () => {
    test("parses single open command", () => {
      const result = parseCommands("1o", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.size).toBe(1);
    });

    test("parses single yank command", () => {
      const result = parseCommands("3y", 10);
      expect(result.get(3)).toEqual(["y"]);
    });

    test("parses single read command", () => {
      const result = parseCommands("5r", 10);
      expect(result.get(5)).toEqual(["r"]);
    });

    test("parses single done command", () => {
      const result = parseCommands("7d", 10);
      expect(result.get(7)).toEqual(["d"]);
    });

    test("parses single unsubscribe command", () => {
      const result = parseCommands("2u", 10);
      expect(result.get(2)).toEqual(["u"]);
    });
  });

  describe("multi-digit indices", () => {
    test("parses double-digit index", () => {
      const result = parseCommands("11o", 20);
      expect(result.get(11)).toEqual(["o"]);
    });

    test("parses triple-digit index", () => {
      const result = parseCommands("123d", 200);
      expect(result.get(123)).toEqual(["d"]);
    });
  });

  describe("multiple actions for same index (no spaces)", () => {
    test("parses number followed by multiple actions: 1od", () => {
      const result = parseCommands("1od", 10);
      expect(result.get(1)).toEqual(["o", "d"]);
      expect(result.size).toBe(1);
    });

    test("parses malformed input: 11oooyd", () => {
      const result = parseCommands("11oooyd", 20);
      expect(result.get(11)).toEqual(["o", "o", "o", "y", "d"]);
      expect(result.size).toBe(1);
    });

    test("parses all actions for one index: 1oyrdu", () => {
      const result = parseCommands("1oyrdu", 10);
      expect(result.get(1)).toEqual(["o", "y", "r", "d", "u"]);
    });
  });

  describe("multiple indices without spaces", () => {
    test("parses 1o3d as two separate commands", () => {
      const result = parseCommands("1o3d", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(3)).toEqual(["d"]);
      expect(result.size).toBe(2);
    });

    test("parses 1o2r3d as three separate commands", () => {
      const result = parseCommands("1o2r3d", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(2)).toEqual(["r"]);
      expect(result.get(3)).toEqual(["d"]);
      expect(result.size).toBe(3);
    });

    test("parses complex sequence 1od3yr5u", () => {
      const result = parseCommands("1od3yr5u", 10);
      expect(result.get(1)).toEqual(["o", "d"]);
      expect(result.get(3)).toEqual(["y", "r"]);
      expect(result.get(5)).toEqual(["u"]);
      expect(result.size).toBe(3);
    });
  });

  describe("notification count bounds checking", () => {
    test("ignores index 0", () => {
      const result = parseCommands("0o", 10);
      expect(result.size).toBe(0);
    });

    test("ignores negative conceptual indices (leading zero)", () => {
      const result = parseCommands("0o1d", 10);
      expect(result.get(1)).toEqual(["d"]);
      expect(result.size).toBe(1);
    });

    test("ignores indices beyond notification count", () => {
      const result = parseCommands("5o", 3);
      expect(result.size).toBe(0);
    });

    test("accepts index equal to notification count", () => {
      const result = parseCommands("3o", 3);
      expect(result.get(3)).toEqual(["o"]);
    });

    test("accepts index 1 with notification count 1", () => {
      const result = parseCommands("1o", 1);
      expect(result.get(1)).toEqual(["o"]);
    });

    test("mixed valid and invalid indices", () => {
      const result = parseCommands("1o5d3r", 3);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(3)).toEqual(["r"]);
      expect(result.has(5)).toBe(false);
      expect(result.size).toBe(2);
    });

    test("all indices out of range returns empty map", () => {
      const result = parseCommands("10o20d30r", 5);
      expect(result.size).toBe(0);
    });
  });

  describe("empty and edge cases", () => {
    test("empty string returns empty map", () => {
      const result = parseCommands("", 10);
      expect(result.size).toBe(0);
    });

    test("only actions without numbers returns empty map", () => {
      const result = parseCommands("oyrdu", 10);
      expect(result.size).toBe(0);
    });

    test("only numbers without actions returns empty map", () => {
      const result = parseCommands("123", 200);
      expect(result.size).toBe(0);
    });

    test("zero notification count returns empty map", () => {
      const result = parseCommands("1o2d", 0);
      expect(result.size).toBe(0);
    });
  });

  describe("whitespace and special characters", () => {
    test("spaces reset the current number", () => {
      const result = parseCommands("1 o", 10);
      // "1" then space resets, "o" has no number
      expect(result.size).toBe(0);
    });

    test("spaces between complete commands work", () => {
      const result = parseCommands("1o 3d", 10);
      expect(result.get(1)).toEqual(["o"]);
      // "3d" after space: "3" builds number, "d" applies
      expect(result.get(3)).toEqual(["d"]);
    });

    test("ignores unknown characters", () => {
      const result = parseCommands("1oxz3d", 10);
      // "1o" works, "x" resets, "z" ignored, "3d" works
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(3)).toEqual(["d"]);
    });

    test("newlines reset state", () => {
      const result = parseCommands("1o\n3d", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(3)).toEqual(["d"]);
    });

    test("tabs reset state", () => {
      const result = parseCommands("1o\t3d", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(3)).toEqual(["d"]);
    });
  });

  describe("repeated actions accumulate", () => {
    test("same index appears multiple times in input", () => {
      const result = parseCommands("1o1d", 10);
      // First "1o" -> 1: [o]
      // Then "1d" -> 1: [o, d]
      expect(result.get(1)).toEqual(["o", "d"]);
    });

    test("same index with actions scattered throughout", () => {
      const result = parseCommands("1o2d1r", 10);
      expect(result.get(1)).toEqual(["o", "r"]);
      expect(result.get(2)).toEqual(["d"]);
    });
  });

  describe("realistic user input scenarios", () => {
    test("quick type: open and done same item", () => {
      const result = parseCommands("1od", 50);
      expect(result.get(1)).toEqual(["o", "d"]);
    });

    test("batch process multiple notifications", () => {
      const result = parseCommands("1d2d3d4d5d", 10);
      expect(result.get(1)).toEqual(["d"]);
      expect(result.get(2)).toEqual(["d"]);
      expect(result.get(3)).toEqual(["d"]);
      expect(result.get(4)).toEqual(["d"]);
      expect(result.get(5)).toEqual(["d"]);
    });

    test("open several, mark others as done", () => {
      const result = parseCommands("1o2o5d6d", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(2)).toEqual(["o"]);
      expect(result.get(5)).toEqual(["d"]);
      expect(result.get(6)).toEqual(["d"]);
    });

    test("yank URL then mark done", () => {
      const result = parseCommands("3yd", 10);
      expect(result.get(3)).toEqual(["y", "d"]);
    });

    test("double-digit notifications batch", () => {
      const result = parseCommands("10o11o12d", 20);
      expect(result.get(10)).toEqual(["o"]);
      expect(result.get(11)).toEqual(["o"]);
      expect(result.get(12)).toEqual(["d"]);
    });
  });

  describe("malformed input handling", () => {
    test("action before any number is ignored", () => {
      const result = parseCommands("o1d", 10);
      expect(result.get(1)).toEqual(["d"]);
      expect(result.size).toBe(1);
    });

    test("multiple leading zeros treated as zero", () => {
      const result = parseCommands("001o", 10);
      // 0, then 0, then 1 -> accumulates to 1
      expect(result.get(1)).toEqual(["o"]);
    });

    test("gibberish interspersed with valid commands", () => {
      const result = parseCommands("abc1oxyz2d!!!", 10);
      expect(result.get(1)).toEqual(["o"]);
      expect(result.get(2)).toEqual(["d"]);
    });

    test("very long number gets parsed", () => {
      const result = parseCommands("99999o", 100000);
      expect(result.get(99999)).toEqual(["o"]);
    });

    test("number overflow scenario is handled by bounds check", () => {
      const result = parseCommands("999999999999o", 10);
      expect(result.size).toBe(0);
    });
  });

  describe("action order preservation", () => {
    test("preserves order of actions for same index", () => {
      const result = parseCommands("1oyrd", 10);
      expect(result.get(1)).toEqual(["o", "y", "r", "d"]);
    });

    test("preserves order across repeated index references", () => {
      const result = parseCommands("1o2d1y1r", 10);
      expect(result.get(1)).toEqual(["o", "y", "r"]);
      expect(result.get(2)).toEqual(["d"]);
    });
  });
});
