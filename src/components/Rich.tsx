export default function Rich({ text }: { text: string }) {
  const parts = text.split(/«([^»]*)»/g);
  return (
    <>
      {parts.map((part, i) =>
        i % 2 === 1 ? (
          <span key={i} style={{ color: "var(--phos)" }}>
            {part}
          </span>
        ) : (
          part
        )
      )}
    </>
  );
}
