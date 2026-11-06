import * as core from '@actions/core';
import { stop } from '../usecases/stop';
import { realStopDeps } from './deps';

try {
  await stop(realStopDeps());
} catch (e) {
  core.setFailed(e instanceof Error ? e.message : String(e));
}
