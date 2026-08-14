export function NniDecimalAmount({
  value,
  className = "",
}: {
  value: string;
  className?: string;
}) {
  const match = /^([+-]?\d+)(\.\d+)(.*)$/.exec(value.trim());
  if (!match) return <span className={className || undefined}>{value}</span>;
  return (
    <span className={className || undefined} data-nni-decimal-amount={value} title={value}>
      <span>{match[1]}</span>
      <span className="nni-decimal-fraction">{match[2]}</span>
      {match[3]}
    </span>
  );
}
