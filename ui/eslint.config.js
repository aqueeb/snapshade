// Purity lint for the TypeScript core (D-06, D-19).
// Source: eslint.org/docs/latest/rules/no-restricted-imports (patterns/group/message syntax verified)
import js from '@eslint/js'; import tseslint from 'typescript-eslint';
export default tseslint.config({ ignores: ['dist/**'] }, js.configs.recommended, ...tseslint.configs.recommended, {
  files: ['src/core/**/*.ts'],
  rules: { 'no-restricted-imports': ['error', { patterns: [
    { group: ['@tauri-apps/*', '@tauri-apps/**'], message: 'ui/src/core must not import Tauri (D-06/D-19)' },
    { group: ['**/overlay/**', '**/options/**', '**/theme/**'], message: 'core must not depend on UI layers' },
  ] } ] },
});
