#!/usr/bin/env node
'use strict';

const { spawn } = require('node:child_process');
const { constants } = require('node:os');
const { join } = require('node:path');
const { existsSync } = require('node:fs');
const platform = `${process.platform}-${process.arch}`;
const executable = process.platform === 'win32' ? 'enderpin.exe' : 'enderpin';
const binary = join(__dirname, 'native', platform, executable);
if (!existsSync(binary)) {
  console.error(`No Enderpin binary for ${platform}. Supported: macOS x64/ARM64, Linux x64/ARM64 (glibc), Windows x64.`);
  process.exit(1);
}

const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit' });
// Ctrl-C reaches both processes in the terminal's foreground group. Let the
// native CLI finish its graceful shutdown before the wrapper exits.
process.on('SIGINT', () => {});
process.on('SIGTERM', () => child.kill('SIGTERM'));
child.on('error', (error) => {
  console.error(`Could not start Enderpin: ${error.message}`);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  process.exitCode = code ?? (128 + (constants.signals[signal] ?? 1));
});
