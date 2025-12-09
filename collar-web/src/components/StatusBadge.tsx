/** Status badge component. */

interface StatusBadgeProps {
  label: string;
  value: string;
  variant?: 'default' | 'success' | 'warning' | 'danger';
}

export function StatusBadge({ label, value, variant = 'default' }: StatusBadgeProps) {
  return (
    <div className={`status-badge ${variant}`}>
      <span className="label">{label}</span>
      <span className="value">{value}</span>
    </div>
  );
}
