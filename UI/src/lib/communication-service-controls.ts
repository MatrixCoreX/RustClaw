export type CommunicationServiceAction = "start" | "stop" | "restart" | "reset";

export function serviceControlActions(
  serviceHealthy: boolean,
  allowReset = true,
): CommunicationServiceAction[] {
  const actions: CommunicationServiceAction[] = serviceHealthy
    ? ["restart", "stop"]
    : ["start"];
  if (allowReset) actions.push("reset");
  return actions;
}
