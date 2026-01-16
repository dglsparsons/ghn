import type { Action } from "../types";

const ACTIONS = new Set<string>(["o", "y", "r", "d", "u"]);

function isAction(char: string): char is Action {
  return ACTIONS.has(char);
}

/**
 * Parses a command string into a map of notification indices to their actions.
 *
 * The parser reads character by character:
 * - Digits build up the current index number
 * - Action characters (o, y, r, d, u) apply the accumulated number as an index
 * - When multiple actions follow a number (e.g., "1od"), each action applies to that index
 * - Malformed input like "11oooyd" becomes: 11 -> [o, o, o, y, d]
 *
 * @param input - The raw command string (e.g., "1o3d" or "11oooyd")
 * @param notificationCount - Total notifications available; indices outside 1..count are ignored
 * @returns Map where keys are 1-based indices and values are arrays of actions
 */
export function parseCommands(
  input: string,
  notificationCount: number
): Map<number, Action[]> {
  const result = new Map<number, Action[]>();

  let currentNumber = 0;
  let hasNumber = false;
  let lastWasAction = false;

  for (const char of input) {
    if (char >= "0" && char <= "9") {
      if (lastWasAction) {
        // Starting a new number after an action
        currentNumber = parseInt(char, 10);
      } else {
        // Continue building current number
        currentNumber = currentNumber * 10 + parseInt(char, 10);
      }
      hasNumber = true;
      lastWasAction = false;
    } else if (isAction(char)) {
      if (hasNumber && currentNumber >= 1 && currentNumber <= notificationCount) {
        const existing = result.get(currentNumber) ?? [];
        existing.push(char);
        result.set(currentNumber, existing);
      }
      lastWasAction = true;
    } else {
      // Any non-digit, non-action character resets state
      currentNumber = 0;
      hasNumber = false;
      lastWasAction = false;
    }
  }

  return result;
}
