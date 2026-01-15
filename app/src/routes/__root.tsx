import { TanStackDevtools } from "@tanstack/react-devtools";
import type { QueryClient } from "@tanstack/react-query";
import { createRootRouteWithContext, Outlet } from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import Header from "../components/Header";
import LoginForm from "../components/LoginForm";

import TanStackQueryDevtools from "../integrations/tanstack-query/devtools";
import { useApiKey } from "../lib/auth";

interface MyRouterContext {
	queryClient: QueryClient;
}

function RootComponent() {
	const apiKey = useApiKey();

	if (!apiKey) {
		return <LoginForm />;
	}

	return (
		<>
			<Header />
			<Outlet />
			<TanStackDevtools
				config={{
					position: "bottom-right",
				}}
				plugins={[
					{
						name: "Tanstack Router",
						render: <TanStackRouterDevtoolsPanel />,
					},
					TanStackQueryDevtools,
				]}
			/>
		</>
	);
}

export const Route = createRootRouteWithContext<MyRouterContext>()({
	component: RootComponent,
});
