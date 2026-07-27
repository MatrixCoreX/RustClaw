export type CommunicationServiceAction = "start" | "stop" | "restart";

export function serviceControlActions(
  serviceHealthy: boolean,
): CommunicationServiceAction[] {
  return serviceHealthy ? ["restart", "stop"] : ["start"];
}
