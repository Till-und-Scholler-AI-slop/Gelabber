// The voice modules keep what lives between calls on the voice session
// (src/voice/sessionState.ts), which resetVoiceForTests replaces as a whole.
// A module-level `let` or `var`, or a `const` holding a new object (a Map, a
// Set, a class instance), would outlive that reset and leak from one call or
// test into the next. Option `allow`: names that may be module-level
// variables, as sessionState.ts's `voice` itself.

/** The expression under TypeScript's `as`, `satisfies` and `!`. */
function unwrap(node) {
  let current = node;
  while (
    current &&
    (current.type === "TSAsExpression" ||
      current.type === "TSSatisfiesExpression" ||
      current.type === "TSNonNullExpression")
  )
    current = current.expression;
  return current;
}

/** @type {import("eslint").Rule.RuleModule} */
export default {
  meta: {
    type: "problem",
    docs: {
      description:
        "Voice modules keep their state on the voice session, not in module variables",
    },
    messages: {
      variable:
        "`{{name}}` is module state, which outlives resetVoiceForTests: keep it on the voice session (sessionState.ts).",
      instance:
        "`{{name}}` holds an object created when the module loads, which outlives resetVoiceForTests: keep it on the voice session (sessionState.ts).",
    },
    schema: [
      {
        type: "object",
        properties: {
          allow: { type: "array", items: { type: "string" } },
        },
        additionalProperties: false,
      },
    ],
  },
  create(context) {
    const allowed = new Set(context.options[0]?.allow ?? []);
    const check = (declaration) => {
      if (declaration?.type !== "VariableDeclaration") return;
      for (const declarator of declaration.declarations) {
        const name =
          declarator.id.type === "Identifier"
            ? declarator.id.name
            : context.sourceCode.getText(declarator.id);
        if (allowed.has(name)) continue;
        if (declaration.kind !== "const")
          context.report({
            node: declarator,
            messageId: "variable",
            data: { name },
          });
        else if (unwrap(declarator.init)?.type === "NewExpression")
          context.report({
            node: declarator,
            messageId: "instance",
            data: { name },
          });
      }
    };
    return {
      Program(program) {
        for (const statement of program.body)
          check(
            statement.type === "ExportNamedDeclaration"
              ? statement.declaration
              : statement,
          );
      },
    };
  },
};
