export function NniDecimalAmount({
  value,
  className = "",
  shrinkFraction = true,
}: {
  value: string;
  className?: string;
  shrinkFraction?: boolean;
}) {
  const match = /^([+-]?\d+)(\.\d+)(.*)$/.exec(value.trim());
  if (!match) return <span className={className || undefined}>{value}</span>;
  if (!shrinkFraction) {
    return (
      <span
        className={className || undefined}
        data-nni-decimal-amount={value}
        data-nni-decimal-fraction-size="normal"
        title={value}
      >
        {value}
      </span>
    );
  }
  return (
    <span
      className={className || undefined}
      data-nni-decimal-amount={value}
      data-nni-decimal-fraction-size="compact"
      title={value}
    >
      <span>{match[1]}</span>
      <span className="nni-decimal-fraction">{match[2]}</span>
      {match[3]}
    </span>
  );
}
