// Text people read goes through the string catalogs (src/i18n, docs/i18n.md),
// so it exists in German and English. This rule finds literals that look
// like such text:
//   1. JSX text with a letter;
//   2. string literals in the JSX attributes below (ATTRIBUTES);
//   3. string literals in object properties named as in PROPERTIES that
//      contain an umlaut, ß, „“ or …, or a capitalised word followed by
//      more words.
// `// i18n-ok` (or `{/* i18n-ok */}`) on the line exempts an intentional
// literal, such as the name "Gelabber".

const ATTRIBUTES = new Set([
  "aria-label",
  "title",
  "placeholder",
  "alt",
  "label",
  "description",
  "hint",
  "message",
]);

const PROPERTIES = new Set([
  "label",
  "description",
  "hint",
  "title",
  "message",
  "text",
  "error",
]);

const LETTER = /\p{L}/u;
const GERMAN = /[äöüÄÖÜß„“…]/;
const SENTENCE = /\p{Lu}\p{Ll}*\s+\p{L}/u;

/** The text of a string literal or a template literal, else null. */
function literalText(node) {
  if (!node) return null;
  if (node.type === "Literal" && typeof node.value === "string")
    return node.value;
  if (node.type === "TemplateLiteral")
    return node.quasis.map((quasi) => quasi.value.cooked ?? "").join(" ");
  if (
    node.type === "JSXExpressionContainer" ||
    node.type === "TSAsExpression" ||
    node.type === "TSSatisfiesExpression"
  )
    return literalText(node.expression);
  return null;
}

function propertyName(node) {
  if (node.computed) return null;
  if (node.key.type === "Identifier") return node.key.name;
  if (node.key.type === "Literal") return String(node.key.value);
  return null;
}

/** @type {import("eslint").Rule.RuleModule} */
export default {
  meta: {
    type: "problem",
    docs: {
      description:
        "UI text goes through the string catalogs (src/i18n/catalogs), in German and English",
    },
    messages: {
      literal:
        "UI text belongs in a string catalog (src/i18n/catalogs, docs/i18n.md). Mark an intentional literal with // i18n-ok.",
    },
    schema: [],
  },
  create(context) {
    const source = context.sourceCode;
    // Lines that carry an `i18n-ok` comment.
    const exempt = new Set();
    for (const comment of source.getAllComments())
      if (/\bi18n-ok\b/.test(comment.value))
        for (
          let line = comment.loc.start.line;
          line <= comment.loc.end.line;
          line++
        )
          exempt.add(line);

    /** The lines of `node` that hold its text, without surrounding blanks. */
    function textLines(node) {
      const raw = source.getText(node);
      const start = raw.search(/\S/);
      const end = raw.trimEnd().length;
      const lines = [];
      if (start < 0) return lines;
      const first = source.getLocFromIndex(node.range[0] + start).line;
      const last = source.getLocFromIndex(node.range[0] + end - 1).line;
      for (let line = first; line <= last; line++) lines.push(line);
      return lines;
    }

    function report(node) {
      if (textLines(node).some((line) => exempt.has(line))) return;
      context.report({ node, messageId: "literal" });
    }

    return {
      JSXText(node) {
        if (LETTER.test(node.value)) report(node);
      },
      JSXAttribute(node) {
        if (node.name.type !== "JSXIdentifier") return;
        if (!ATTRIBUTES.has(node.name.name)) return;
        const text = literalText(node.value);
        if (text !== null && LETTER.test(text)) report(node.value);
      },
      Property(node) {
        const name = propertyName(node);
        if (!name || !PROPERTIES.has(name)) return;
        const text = literalText(node.value);
        if (text !== null && (GERMAN.test(text) || SENTENCE.test(text)))
          report(node.value);
      },
    };
  },
};
