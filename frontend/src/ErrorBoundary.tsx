import { Component, type ReactNode } from 'react';

/** Keeps a failing panel from blanking the whole app; shows the error with a retry. */
export class ErrorBoundary extends Component<{ children: ReactNode; label: string }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    console.error(`[${this.props.label}]`, error);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="panel pad">
        <div className="error">
          {this.props.label} failed to display: {this.state.error.message}
        </div>
        <button onClick={() => this.setState({ error: null })}>Retry</button>
      </div>
    );
  }
}
