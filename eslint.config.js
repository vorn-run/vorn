import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import vitest from '@vitest/eslint-plugin'
import prettier from 'eslint-config-prettier'

export default tseslint.config(
  {
    ignores: [
      'out/',
      'dist/',
      '**/dist/',
      'node_modules/',
      '*.config.js',
      '**/*.config.cjs',
      'scripts/',
      'packages/core/scripts/',
      'apps/macos/scripts/',
      'apps/macos/Sources/VornUI/Resources/shiki.js',
      'packages/core/target/',
      'website/scripts/',
      'spikes/'
    ]
  },

  js.configs.recommended,
  ...tseslint.configs.recommended,
  prettier,

  {
    rules: {
      '@typescript-eslint/no-unused-vars': [
        'warn',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }
      ],
      '@typescript-eslint/no-explicit-any': 'warn'
    }
  },

  {
    files: ['tests/**/*.test.ts'],
    plugins: { vitest },
    languageOptions: {
      globals: {
        ...vitest.environments.env.globals
      }
    },
    rules: {
      // Core anti-fake-test guardrails. Keep these as errors so a PR can't
      // merge a test that makes no assertions, a .only that silences the
      // rest of the suite, a .skip, or two tests that shadow each other.
      'vitest/expect-expect': 'error',
      'vitest/no-focused-tests': 'error',
      'vitest/no-disabled-tests': 'error',
      'vitest/no-identical-title': 'error',
      'vitest/no-commented-out-tests': 'error',
      'vitest/valid-expect': 'error'
    }
  }
)
