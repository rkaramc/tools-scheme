import { defineConfig } from '@vscode/test-cli';

export default defineConfig({
  files: 'out/tests/**/*.integration.test.js',
  launchArgs: [
    'D:\\source\\learn-scheme\\little-schemer',
    '--disable-extensions',
    '--no-sandbox',
    '--disable-gpu-sandbox'
  ]
});
