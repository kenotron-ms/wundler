// CJS fixture module — used by cjs::stub tests.
const helper = (x) => x * 2;

module.exports = { double: helper, triple: (x) => x * 3, VERSION: '1.0.0' };
