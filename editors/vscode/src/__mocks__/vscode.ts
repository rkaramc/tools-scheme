export const workspace = {
    getConfiguration: jest.fn().mockReturnValue({
        get: jest.fn()
    })
};
export const window = {
    createOutputChannel: jest.fn().mockReturnValue({
        appendLine: jest.fn()
    }),
    showErrorMessage: jest.fn(),
    showInformationMessage: jest.fn()
};
export const commands = {
    registerCommand: jest.fn()
};
export const Uri = {
    parse: jest.fn()
};
export enum ExtensionMode {
    Production = 1,
    Development = 2,
    Test = 3
}
