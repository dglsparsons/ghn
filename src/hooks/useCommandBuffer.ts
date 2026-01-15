import { useState } from "react";
import type { Action, Command, CommandBufferState } from "../types";

export function useCommandBuffer() {
  const [state, setState] = useState<CommandBufferState>({
    raw: "",
    commands: [],
    pendingNumber: null,
  });

  const addDigit = (digit: number) => {
    setState((prev) => {
      const nextNumber = prev.pendingNumber === null ? digit : prev.pendingNumber * 10 + digit;
      return {
        ...prev,
        pendingNumber: nextNumber,
        raw: prev.raw + String(digit),
      };
    });
  };

  const addAction = (action: Action) => {
    setState((prev) => {
      const index = prev.pendingNumber;
      const nextCommands = index !== null ? [...prev.commands, { index, action } as Command] : prev.commands;

      const needsSpace = prev.raw.length > 0 && !prev.raw.endsWith(" ");
      const nextRaw = (needsSpace ? prev.raw + " " : prev.raw) + action + " ";

      return {
        raw: nextRaw,
        commands: nextCommands,
        pendingNumber: null,
      };
    });
  };

  const clear = () => {
    setState({ raw: "", commands: [], pendingNumber: null });
  };

  const backspace = () => {
    setState((prev) => {
      if (prev.raw.length === 0) return prev;

      const nextRaw = prev.raw.slice(0, -1);

      if (prev.pendingNumber !== null) {
        const nextNumber = Math.floor(prev.pendingNumber / 10);
        return {
          ...prev,
          raw: nextRaw,
          pendingNumber: nextNumber > 0 ? nextNumber : null,
        };
      }

      return {
        ...prev,
        raw: nextRaw,
      };
    });
  };

  return {
    state,
    addDigit,
    addAction,
    clear,
    backspace,
  };
}
