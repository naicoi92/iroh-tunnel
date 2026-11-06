import * as core from '@actions/core';
import { runMain } from '../usecases/run-main';
import { realStartDeps } from './deps';

try {
  await runMain('serve', realStartDeps());
} catch (e) {
  core.setFailed(e instanceof Error ? e.message : String(e));
}
