module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  modulePathIgnorePatterns: ["<rootDir>/out/"],
  testPathIgnorePatterns: ["<rootDir>/src/tests/extension.integration.test.ts"],
  testMatch: ["<rootDir>/src/**/*.test.ts"]
};
