import usageDoc from "../../docs/tdl-desktop-usage.md?raw";

export function DocumentationWorkspace({ title }: { title: string }) {
  return (
    <section className="docs-page">
      <div className="section-header">
        <h2>{title}</h2>
      </div>
      <MarkdownDocument content={usageDoc} />
    </section>
  );
}

function MarkdownDocument({ content }: { content: string }) {
  return (
    <div className="markdown-doc">
      {content.split(/\n{2,}/).map((block, index) => {
        const text = block.trim();
        if (!text) return null;
        if (text.startsWith("# ")) return <h1 key={index}>{text.slice(2)}</h1>;
        if (text.startsWith("## ")) return <h2 key={index}>{text.slice(3)}</h2>;
        if (text.startsWith("### ")) return <h3 key={index}>{text.slice(4)}</h3>;
        if (text.startsWith("```")) return <pre key={index}>{text.replace(/^```\w*\n?/, "").replace(/```$/, "")}</pre>;
        if (text.split("\n").every((line) => line.startsWith("- "))) {
          return (
            <ul key={index}>
              {text.split("\n").map((line) => <li key={line}>{line.slice(2)}</li>)}
            </ul>
          );
        }
        if (/^\d+\. /.test(text)) {
          return (
            <ol key={index}>
              {text.split("\n").map((line) => <li key={line}>{line.replace(/^\d+\. /, "")}</li>)}
            </ol>
          );
        }
        return <p key={index}>{text}</p>;
      })}
    </div>
  );
}
