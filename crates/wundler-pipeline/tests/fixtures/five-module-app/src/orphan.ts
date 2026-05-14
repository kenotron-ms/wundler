// This module is intentionally not imported by any other module.
// It is here to test that the dead-code elimination pipeline correctly
// identifies it as unreachable.
export const orphan = 99;
