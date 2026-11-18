TERMUST-MSG(1)

# NAME

termust-msg - Send messages to Termust.

# SYNOPSIS

This command communicates with running Termust instances through a socket,
making it possible to control Termust without directly accessing it.

# OPTIONS

*-s, --socket* _<SOCKET>_

	Path for IPC socket communication.

# MESSAGES

*create-window*

	Create a new window in the same Termust process.

	*FLAGS*
		*--hold*

			Remain open after child process exits.

	*OPTIONS*
		*--working-directory* _<WORKING_DIRECTORY>_

			Start the shell in the specified working directory.

		*-T, --title* _<TITLE>_

			Defines the window title.

			Default: _Termust_

		*--class* _<GENERAL>_ | _<GENERAL>_,_<INSTANCE>_

			Defines window class/app_id on X11/Wayland.

			Default: _Termust,Termust_

		*-o, --option* _<OPTION>..._

			Override configuration file options.

			Example: _termust msg create-window -o 'cursor.style="Beam"'_

		*-e, --command* _<COMMAND>..._

			Command and args to execute (must be last argument).

*config*

	Update the Termust configuration.

	*ARGS*
		*<CONFIG_OPTIONS>...*

			Configuration file options.

			Example: _termust msg config 'cursor.style="Beam"'_

	*FLAGS*
		*-r, --reset*

			Clear all runtime configuration changes.

	*OPTIONS*
		*-w, --window-id* _<WINDOW_ID>_

			Window ID for the new config.

			Use _-1_ to apply this change to all windows.

			Default: _$TERMUST_WINDOW_ID_

*get-config*

	Read runtime Termust configuration.

	*OPTIONS*
		*-w, --window-id* _<WINDOW_ID>_

			Window ID for the config request.

			Use _-1_ to get the global config.

			Default: _$TERMUST_WINDOW_ID_

# SEE ALSO

*termust*(1), *termust*(5), *termust-bindings*(5), *termust-escapes*(7)

# BUGS

Found a bug? Please report it at _https://github.com/tarcisiomiranda/alacritty/issues_.

# MAINTAINERS

- Christian Duerr <contact@christianduerr.com>
- Kirill Chibisov <contact@kchibisov.com>
