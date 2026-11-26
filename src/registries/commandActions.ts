export interface CommandActionDefinition {
  id: string;
  label: string;
  group: string;
  shortcut?: string;
  keywords?: string[];
  run: () => void | Promise<void>;
}

const actions = new Map<string, CommandActionDefinition>();

export function registerCommandAction(action: CommandActionDefinition): void {
  actions.set(action.id, action);
}

export function listCommandActions(): CommandActionDefinition[] {
  return Array.from(actions.values());
}
