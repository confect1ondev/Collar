/** Script button component. */

interface Script {
  id: string;
  name: string;
  icon?: string;
}

interface ScriptButtonProps {
  script: Script;
  onClick: () => void;
  loading?: boolean;
  disabled?: boolean;
}

export function ScriptButton({ script, onClick, loading, disabled }: ScriptButtonProps) {
  return (
    <button
      className="script-button"
      onClick={onClick}
      disabled={disabled}
      title={script.name}
    >
      {loading ? (
        <span className="spinner" />
      ) : (
        <>
          {script.icon && <span className="icon">{script.icon}</span>}
          <span className="name">{script.name}</span>
        </>
      )}
    </button>
  );
}
