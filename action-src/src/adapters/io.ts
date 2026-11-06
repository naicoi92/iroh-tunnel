import * as realCore from '@actions/core';

export interface ActionsIO {
  input(name: string): string;
  setOutput(k: string, v: string): void;
  setSecret(v: string): void;
  saveState(k: string, v: string): void;
  getState(k: string): string;
  info(msg: string): void;
  warning(msg: string): void;
  group<T>(title: string, fn: () => Promise<T>): Promise<T>;
  setFailed(msg: string): void;
  jobFailed(): boolean;
}

export type CoreLike = {
  getInput(name: string, o?: { trimWhitespace?: boolean }): string;
  setOutput(k: string, v: string): void;
  setSecret(v: string): void;
  saveState(k: string, v: string): void;
  getState(k: string): string;
  info(msg: string): void;
  warning(msg: string): void;
  startGroup(t: string): void;
  endGroup(): void;
  setFailed(msg: string): void;
};

export function createIO(core: CoreLike = realCore): ActionsIO {
  const io: ActionsIO = {
    input: (name) => core.getInput(name).trim(),
    setOutput: (k, v) => core.setOutput(k, v),
    setSecret: (v) => core.setSecret(v),
    saveState: (k, v) => core.saveState(k, v),
    getState: (k) => core.getState(k),
    info: (m) => core.info(m),
    warning: (m) => core.warning(m),
    async group(title, fn) {
      core.startGroup(title);
      try {
        return await fn();
      } finally {
        core.endGroup();
      }
    },
    setFailed: (m) => core.setFailed(m),
    jobFailed() {
      const s = io.input('job-status');
      return s === 'failure' || s === 'cancelled' || core.getState('main-ok') === '';
    },
  };
  return io;
}
