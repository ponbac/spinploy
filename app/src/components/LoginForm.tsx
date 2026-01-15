import { KeyRound } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { setApiKey } from "../lib/auth";

export default function LoginForm() {
	const [apiKey, setLocalApiKey] = useState("");
	const inputId = useId();

	function handleSubmit(e: FormEvent) {
		e.preventDefault();
		if (apiKey.trim()) {
			setApiKey(apiKey.trim());
		}
	}

	return (
		<div className="min-h-screen bg-gray-900 flex items-center justify-center p-4">
			<div className="w-full max-w-md">
				<div className="bg-gray-800 rounded-xl shadow-2xl p-8">
					<div className="flex items-center justify-center gap-3 mb-8">
						<KeyRound className="text-emerald-500" size={32} />
						<h1 className="text-2xl font-bold text-white">Spinploy</h1>
					</div>

					<form onSubmit={handleSubmit} className="space-y-6">
						<div>
							<label
								htmlFor={inputId}
								className="block text-sm font-medium text-gray-300 mb-2"
							>
								Dokploy API Key
							</label>
							<input
								id={inputId}
								type="password"
								value={apiKey}
								onChange={(e) => setLocalApiKey(e.target.value)}
								placeholder="Enter your API key"
								className="w-full px-4 py-3 bg-gray-700 border border-gray-600 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-emerald-500 focus:border-transparent transition-all"
							/>
						</div>

						<button
							type="submit"
							className="w-full py-3 px-4 bg-emerald-600 hover:bg-emerald-500 text-white font-medium rounded-lg transition-colors focus:outline-none focus:ring-2 focus:ring-emerald-500 focus:ring-offset-2 focus:ring-offset-gray-800"
						>
							Sign In
						</button>
					</form>

					<p className="mt-6 text-center text-sm text-gray-400">
						Your API key is stored locally in your browser.
					</p>
				</div>
			</div>
		</div>
	);
}
