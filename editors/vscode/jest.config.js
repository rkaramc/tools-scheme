module.exports = {
  preset: 'ts-jest',
  testEnvironment: 'node',
  modulePathIgnorePatterns: ["<rootDir>/out/"],
  testMatch: ["<rootDir>/src/**/*.test.ts"]
};
