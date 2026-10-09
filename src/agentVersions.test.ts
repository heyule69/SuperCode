import { expect, it } from 'vitest';
import { releaseState } from './agentVersions';
it('compares numerical components and native banners',()=>{
  expect(releaseState('codex-cli 0.160.1','0.162.0')).toBe('update');
  expect(releaseState('2.1.99 (Claude Code)','2.1.295')).toBe('update');
  expect(releaseState('v1.0.4','1.0.4')).toBe('current');
  expect(releaseState('1.0.4+local.2','1.0.4')).toBe('current');
  expect(releaseState('1.0.5','1.0.4')).toBe('newer');
  expect(releaseState('1.0.4-beta.1','1.0.4')).toBe('update');
  expect(releaseState('unknown','1.0.4')).toBe('unknown');
  expect(releaseState('1.0.4',null)).toBe('unknown');
});
