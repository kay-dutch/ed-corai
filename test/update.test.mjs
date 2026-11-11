// Version comparison and repository detection used by the release check.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const { compareVersions, parseGithubSlug } = createRequire(import.meta.url)('../dist/testkit.cjs');

test('compares semantic versions', () => {
  assert.equal(compareVersions('0.2.0', '0.1.9'), 1);
  assert.equal(compareVersions('0.10.0', '0.9.9'), 1);
  assert.equal(compareVersions('v1.0.0', '1.0.0'), 0);
  assert.equal(compareVersions('1.0.0', '1.0.1'), -1);
  assert.equal(compareVersions('1.0.0-rc.1', '1.0.0'), -1);
  assert.equal(compareVersions('1.0.0-beta.10', '1.0.0-beta.2'), 1);
  assert.equal(compareVersions('1.0', '1.0.0'), 0);
});

test('reads GitHub repositories from remote URLs', () => {
  for (const url of [
    'https://github.com/kay-dutch/ed-corai.git',
    'https://github.com/kay-dutch/ed-corai',
    'git@github.com:kay-dutch/ed-corai.git',
    'git+https://github.com/kay-dutch/ed-corai.git',
  ]) assert.equal(parseGithubSlug(url), 'kay-dutch/ed-corai', url);
  assert.equal(parseGithubSlug('https://gitlab.com/someone/project.git'), undefined);
  assert.equal(parseGithubSlug(undefined), undefined);
});
