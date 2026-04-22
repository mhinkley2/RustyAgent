/** Shared placeholder used by all page stubs while real pages are built. */
export default function PlaceholderPage({ title }: { title: string }) {
  return (
    <div className="placeholder-page">
      <p className="placeholder-page__label">Coming soon</p>
      <h1 className="placeholder-page__title">{title}</h1>
      <p className="placeholder-page__hint">
        This page will be implemented in an upcoming sprint.
      </p>
    </div>
  );
}
